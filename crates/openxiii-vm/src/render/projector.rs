//! Projectors (Engine.dll's AProjector): bullet impacts, blood, hand light.
//!
//! AProjector::CalcMatrix computes the projection frustum and the matrices;
//! AProjector::Attach creates an FProjectorRenderInfo and attaches it to the
//! BSP nodes it touches (UModel::ConvexVolumeMultiCheck, UModel::AttachProjector); the BSP renderer draws these nodes with the projector's material
//! as long as considers the info valid.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::Value;
use crate::{ObjId, Vm};
use openxiii_assets::coords;
use std::rc::Rc;

type V = [f32; 3];

/// FProjectorRenderInfo (0x9C bytes).
#[derive(Debug, Clone)]
pub struct ProjInfo {
    /// Lifetime identity, with no effect on the simulation rules.
    pub geometry_life: Rc<()>,
    /// +0: references (the projector and each node).
    refs: i32,
    /// +4: last render (real seconds); +0xC: end of life (0: never).
    last_render: f64,
    expiry: f64,
    /// +0x14: ProjTexture.
    pub material: Value,
    /// +0x18: Matrix; +0x58: GradientMatrix (rows, row vector).
    pub matrix: [f32; 16],
    pub gradient: [f32; 16],
    /// +0x98: 1 FOV ≠ 0 (projected coordinates), 2 bProjectOnUnlit, 4 bFade, 8 bProjectOnAlpha.
    pub flags: u16,
    /// +0x9A: AttachPriority.
    priority: u8,
    /// Source projector (for the shadow textures).
    pub projector: ObjId,
}

/// Native state of an AProjector (the script's transient fields).
#[derive(Debug, Clone, Default)]
pub struct ProjectorState {
    planes: [[f32; 4]; 6],
    vertices: [V; 8],
    bbox: Option<(V, V)>,
    matrix: [f32; 16],
    gradient: [f32; 16],
    /// +0x374: RenderInfo.
    info: Option<usize>,
}

/// FBspNodeProjectorInfo (0x10 bytes): info and clipped polygon (empty: the node's).
#[derive(Debug, Clone)]
pub struct NodeProjector {
    info: usize,
    poly: Rc<[V]>,
}

/// Entry of UTerrainSector+0xF4 (0xC bytes): info and FRawIndexBuffer of the triangles touched
/// (the sector's vertices; the smallest and largest indices of the entry are only used for the
/// DrawPrimitive).
#[derive(Debug, Clone)]
pub struct SectorProjector {
    info: usize,
    indices: Vec<u16>,
    poly: std::cell::OnceCell<Rc<[V]>>,
}

/// Entry of Actor.Projectors (0xC bytes): info and, for a StaticMesh attached by a fixed
/// projector (bStatic or !bMovable), the triangles touched (otherwise all the projectable sections).
#[derive(Debug, Clone)]
pub struct ActorProjector {
    info: usize,
    indices: Option<Vec<u16>>,
    /// Clipped triangles in world coordinates, as long as the mesh and the matrix are identical.
    indexed_poly: Option<(Rc<openxiii_assets::static_mesh::StaticMesh>, [[u32; 4]; 4], Rc<[V]>)>,
}

/// Native data of an ATerrainInfo read for the projectors.
#[derive(Debug, Clone)]
pub struct TerrainCache {
    pub data: openxiii_assets::terrain::TerrainData,
    /// EdgeTurnBitmap (+0x82C), at the size of the heightmap.
    pub edge_turn: Vec<u32>,
}

/// A projector to draw on a BSP node or a terrain sector.
#[derive(Debug, Clone)]
pub struct ProjectorDraw {
    /// Receiver type, actor/node, sector: independent of the visible rank.
    pub fragment: [usize; 3],
    pub node: usize,
    /// Clipped polygon, otherwise the node's (in the order of the BSP vertex stream); with
    /// `triangles`, a triangle list (terrain sector).
    pub poly: std::rc::Rc<[V]>,
    /// Terrain triangles (ATerrainInfo::Render: without SetZBias).
    pub triangles: bool,
    /// Path of ProjTexture.
    pub texture: String,
    pub info: ProjInfo,
}

#[derive(Default)]
pub struct Projectors {
    infos: Vec<Option<ProjInfo>>,
    states: std::collections::HashMap<ObjId, ProjectorState>,
    nodes: std::collections::BTreeMap<usize, Vec<NodeProjector>>,
    /// BSP vertices without clipping, shared by the projectors of the same node.
    pub(crate) node_polys: std::collections::HashMap<usize, Rc<[V]>>,
    /// Keys in BTreeMap order while removing the stale entries.
    draw_nodes: Vec<usize>,
    draw_actors: Vec<ObjId>,
    draw_sectors: Vec<(ObjId, usize)>,
    /// Terrains of the map (read at the first attachment) and the projectors of their sectors.
    terrains: Option<std::collections::HashMap<ObjId, TerrainCache>>,
    sectors: std::collections::BTreeMap<(ObjId, usize), Vec<SectorProjector>>,
    /// Actor.Projectors of the actors and the StaticMeshes read for them.
    actors: std::collections::BTreeMap<ObjId, Vec<ActorProjector>>,
    meshes: std::collections::HashMap<String, Option<std::rc::Rc<openxiii_assets::static_mesh::StaticMesh>>>,
    /// StaticMesh+0x184 of each StaticMesh (number of RawTriangles after PostLoad), read once.
    raw_triangles: std::collections::HashMap<String, i16>,
    /// Counter of shadow updates (one or two per frame).
    shadow_counter: u32,
    /// Silhouettes to (re)draw this frame (UShadowBitmapMaterial vt 0x94 then Update).
    pub shadow_jobs: Vec<ShadowJob>,
    /// UShadowBitmapMaterials already drawn (vt 0x8C): shadow texture per projector.
    pub shadow_valid: std::collections::HashSet<ObjId>,
    /// ShadowBitmapMaterial +0x78 of each shadow: its intensity, written by UpdateShadow on
    /// every call and read when the projection is drawn (TFACTOR).
    shadow_intensity: std::collections::HashMap<ObjId, i32>,
    /// Eight 256 × 256 targets of D3DDrv: the projector that occupies it and
    /// the number of its last update (takes a free target, otherwise the one
    /// updated longest ago).
    /// More shadow targets (see `shadow_slot_count`): grows as shadows need targets, the first
    /// eight otherwise.
    shadow_slots: Vec<Option<(ObjId, u32)>>,
    /// Graphics: Enhanced (improved build), set by the viewer: more shadow targets in a single
    /// view too (`shadow_slot_count`).
    pub more_shadow_targets: bool,
    /// Seconds added to app_seconds for the projectors' clock (`projector_now`).
    pub(crate) clock_origin: f64,
    /// Split screen: the frame and the views (bits) that asked for shadows in it, and the last
    /// frame where more than one view did (`shadow_slot_count`).
    view_mask: (u32, u32),
    split_frame: Option<u32>,
    shadow_stamp: u32,
    /// Split screen (co-op, several viewports): frame and distance of the last UpdateShadow
    /// request of each shadow in each view (index = viewport slot).
    shadow_views: std::collections::HashMap<ObjId, Vec<Option<(u32, f32)>>>,
    /// Projectable triangles of an actor (without projector-specific indices) in world
    /// coordinates, kept as long as its StaticMesh and its LocalToWorld are the same.
    actor_polys: std::collections::HashMap<ObjId, (String, [[u32; 4]; 4], std::rc::Rc<[V]>)>,
}

/// Light direction of a shadow (AShadowProjector::UpdateShadow):
/// the instance's for DrawType 2 (mesh) or 8 (StaticMesh), otherwise (0, 0, 1). Light too
/// grazing (Z < 0.707106): Z = 0.707106 and XY brought to 0.707106 / |XY|. 24-bit x87 computation:
/// squares, sum, root (msvcr70), quotient and products rounded to single precision (the double
/// is exactly the float 0x3F3504E6).
pub(crate) fn shadow_light_dir(draw_type: u8, instance_dir: Option<V>) -> V {
    let mut dir = match (draw_type, instance_dir) {
        (2 | 8, Some(d)) => d,
        _ => return [0.0, 0.0, 1.0],
    };
    let c = f32::from_bits(0x3f35_04e6);
    if dir[2] < c {
        dir[2] = c;
        let k = c / (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
        dir[0] *= k;
        dir[1] *= k;
    }
    dir
}

/// Attenuation of a shadow with distance: 1 up to 1000, then
/// 1 - (dist - ShadowTransDist) / (ShadowMaxDist - ShadowTransDist), not below 0.
pub(crate) fn shadow_fade(dist: f32, max: f32, trans: f32) -> f32 {
    if !(dist > 1000.0) {
        return 1.0;
    }
    let f = 1.0 - (dist - trans) / (max - trans);
    if f < 0.0 {
        0.0
    } else {
        f
    }
}

/// Update of the shadow texture: `counter` catches up with frame `f`;
/// UpdateCount ≤ f, or > f + 100 (counter restarted), gives an update: new UpdateCount.
pub(crate) fn shadow_schedule(counter: &mut u32, count: u32, f: u32) -> Option<u32> {
    if *counter < f {
        *counter = f;
    }
    if count <= f {
        *counter = counter.wrapping_add(1);
    } else if count > f.wrapping_add(100) {
        *counter = f.wrapping_add(1);
    } else {
        return None;
    }
    Some(*counter)
}

/// D3DDrv's shadow targets.
pub const SHADOW_SLOTS: usize = 8;

/// Shadow targets the renderer could make, when it failed to make one more (out of video
/// memory): the targets in use stop there and 's reuse of the oldest one takes
/// over. usize::MAX while no creation has failed.
static SHADOW_TARGET_LIMIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

/// The renderer could not make shadow target `made` (it has `made` targets): no more targets
/// are given out beyond them.
pub fn note_shadow_target_limit(made: usize) {
    SHADOW_TARGET_LIMIT.fetch_min(made.max(SHADOW_SLOTS), std::sync::atomic::Ordering::Relaxed);
}

/// Origin of the projectors' clock (`Vm::projector_now`): processor uptime when the game starts.
/// Only its being well beyond one second matters to the lifetime rules.
pub(crate) const CLOCK_ORIGIN: f64 = 3600.0;

impl Projectors {
    /// No projector, the clock at the game's start (`CLOCK_ORIGIN`).
    pub(crate) fn new() -> Self {
        Self { clock_origin: CLOCK_ORIGIN, ..Default::default() }
    }
}

/// Shadow targets in use: D3DDrv's eight, the original's single view. In split screen the views
/// and the extra enemies, and with Graphics: Enhanced the longer shadow distance or many
/// characters (mods), easily give more than eight shadows at once: each character's shadow,
/// drawn again every frame, then took the target updated longest ago, that of another shadow,
/// whose projection was skipped until it took a target back on a later frame (shadows
/// blinking or missing). There the targets have no fixed bound: a new one is made when every
/// target is held, up to what the video memory allows (`note_shadow_target_limit`).
pub(crate) fn shadow_slot_count(split: bool, enhanced: bool) -> usize {
    if (split || enhanced) {
        SHADOW_TARGET_LIMIT.load(std::sync::atomic::Ordering::Relaxed)
    } else {
        SHADOW_SLOTS
    }
}

/// Among the first `used` targets: the one `sp` already holds, otherwise a free
/// one, otherwise a new one while fewer than `used` exist, otherwise the one updated longest
/// ago. Returns it with the projectors that lost their target (the one taken, and those
/// beyond `used` when it shrank, back to a single view in Graphics: Original).
pub(crate) fn shadow_slot_take(slots: &mut Vec<Option<(ObjId, u32)>>, used: usize, sp: ObjId, stamp: u32) -> (usize, Vec<ObjId>) {
    let used = used.max(1);
    let mut lost: Vec<ObjId> = Vec::new();
    if slots.len() > used {
        lost.extend(slots.drain(used..).flatten().map(|(p, _)| p));
    }
    let slot = match slots.iter().position(|s| s.is_some_and(|(p, _)| p == sp)) {
        Some(i) => i,
        None => match slots.iter().position(|s| s.is_none()) {
            Some(i) => i,
            None if slots.len() < used => {
                slots.push(None);
                slots.len() - 1
            }
            None => {
                let (i, _) = slots.iter().enumerate().min_by_key(|(_, s)| s.map_or(0, |(_, t)| t)).unwrap();
                lost.extend(slots[i].map(|(p, _)| p));
                i
            }
        },
    };
    slots[slot] = Some((sp, stamp));
    lost.retain(|&p| p != sp);
    (slot, lost)
}

/// Records that `view` draws `frame` (`mask` = the frame and its views, bits); `split` = the
/// last frame drawn by more than one view.
pub(crate) fn note_shadow_view(mask: &mut (u32, u32), split: &mut Option<u32>, frame: u32, view: u8) {
    if mask.0 != frame {
        *mask = (frame, 0);
    }
    mask.1 |= 1 << (view % 32);
    if mask.1.count_ones() > 1 {
        *split = Some(frame);
    }
}

#[cfg(test)]
#[test]
fn split_screen_is_two_views_in_a_frame() {
    let (mut mask, mut split) = ((0, 0), None);
    for f in 1..10 {
        note_shadow_view(&mut mask, &mut split, f, 0);
    }
    assert_eq!(split, None);
    // Viewports 0 and 3 (the second player's) in frame 10.
    note_shadow_view(&mut mask, &mut split, 10, 0);
    note_shadow_view(&mut mask, &mut split, 10, 3);
    assert_eq!(split, Some(10));
    note_shadow_view(&mut mask, &mut split, 11, 0);
    assert_eq!(split, Some(10));
}

#[cfg(test)]
#[test]
fn coop_shadows_keep_their_targets_beyond_eight() {
    // Six characters' shadows drawn again every frame, three static ones drawn once (vehicles),
    // as in Base01's hangar seen by the second player with the extra enemies.
    // Solo, Graphics: Original: the original's eight targets, the static shadows lose theirs.
    assert!(shadow_target_losses(6, 3, false, false) > 50);
    // Co-op: as many targets as the shadows need, no shadow loses its target.
    assert_eq!(shadow_target_losses(6, 3, true, false), 0);
    assert_eq!(shadow_target_losses(6, 3, true, true), 0);
    // A target already held is kept; back to a single view in Graphics: Original, the targets
    // beyond eight are freed.
    let mut slots = Vec::new();
    for p in 0..12 {
        assert_eq!(shadow_slot_take(&mut slots, usize::MAX, p, p as u32 + 1), (p, vec![]));
    }
    assert_eq!(slots.len(), 12);
    assert_eq!(shadow_slot_take(&mut slots, usize::MAX, 3, 50), (3, vec![]));
    assert_eq!(shadow_slot_take(&mut slots, SHADOW_SLOTS, 3, 51), (3, vec![8, 9, 10, 11]));
    assert_eq!(slots.len(), SHADOW_SLOTS);
    // Solo with eight held: the oldest (0) gives its target to a ninth.
    assert_eq!(shadow_slot_take(&mut slots, SHADOW_SLOTS, 20, 52), (0, vec![0]));
}

/// Shadows that lose their target over 50 frames: `characters` shadows drawn again every
/// frame, `statics` drawn once and again only after losing their target (vehicles, corpses).
#[cfg(test)]
fn shadow_target_losses(characters: usize, statics: usize, split: bool, enhanced: bool) -> usize {
    let mut slots = Vec::new();
    let (mut stamp, mut taken) = (0, 0);
    let statics: Vec<ObjId> = (0..statics).map(|v| 10_000 + v).collect();
    for &v in &statics {
        stamp += 1;
        shadow_slot_take(&mut slots, shadow_slot_count(split, enhanced), v, stamp);
    }
    let mut valid: std::collections::HashSet<ObjId> = statics.iter().copied().collect();
    for _ in 0..50 {
        for c in 0..characters {
            stamp += 1;
            let (_, lost) = shadow_slot_take(&mut slots, shadow_slot_count(split, enhanced), c, stamp);
            for p in lost {
                valid.remove(&p);
                taken += 1;
            }
        }
        for &v in &statics {
            if !valid.contains(&v) {
                stamp += 1;
                let (_, lost) = shadow_slot_take(&mut slots, shadow_slot_count(split, enhanced), v, stamp);
                valid.insert(v);
                for p in lost {
                    valid.remove(&p);
                    taken += 1;
                }
            }
        }
    }
    taken
}

#[cfg(test)]
#[test]
fn enhanced_shadows_keep_their_targets_beyond_eight() {
    // Graphics: Enhanced, a single view, many characters at once (a mod spawning dozens of
    // singing SPADS): every count of shadows from 9 to 120 keeps its targets, the targets made
    // as needed.
    for n in [9, 12, 16, 20, 31, 33, 50, 120] {
        assert_eq!(shadow_target_losses(n, 2, false, true), 0, "{n} characters");
        let mut slots = Vec::new();
        for p in 0..n {
            assert_eq!(shadow_slot_take(&mut slots, shadow_slot_count(false, true), p, p as u32 + 1), (p, vec![]));
        }
        assert_eq!(slots.len(), n);
    }
    // Eight or fewer: the same targets as the original, whatever the mode.
    for n in 1..=SHADOW_SLOTS {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for p in 0..n {
            assert_eq!(shadow_slot_take(&mut a, shadow_slot_count(false, true), p, p as u32 + 1), shadow_slot_take(&mut b, SHADOW_SLOTS, p, p as u32 + 1));
        }
        assert_eq!(a, b);
    }
}

#[cfg(test)]
#[test]
fn original_solo_shadows_still_take_the_oldest_target() {
    // Graphics: Original, a single view: D3DDrv's eight targets, a ninth shadow takes the one
    // updated longest ago, never a ninth target.
    for n in [9, 12, 16, 40] {
        assert!(shadow_target_losses(n, 2, false, false) > 0, "{n} characters");
        let mut slots = Vec::new();
        for p in 0..n {
            let (slot, lost) = shadow_slot_take(&mut slots, shadow_slot_count(false, false), p, p as u32 + 1);
            if p < SHADOW_SLOTS {
                assert_eq!((slot, lost), (p, vec![]));
            } else {
                assert_eq!((slot, lost), ((p - SHADOW_SLOTS) % SHADOW_SLOTS, vec![p - SHADOW_SLOTS]));
            }
        }
        assert_eq!(slots.len(), SHADOW_SLOTS);
    }
}

#[cfg(test)]
#[test]
fn more_shadow_targets_stop_at_the_video_memory_limit() {
    // Within a limit (a target the renderer could not make): the oldest target is taken again,
    // as in the original, and no target beyond the limit is given out.
    let mut slots = Vec::new();
    for p in 0..10 {
        assert_eq!(shadow_slot_take(&mut slots, 10, p, p as u32 + 1), (p, vec![]));
    }
    assert_eq!(shadow_slot_take(&mut slots, 10, 10, 11), (0, vec![0]));
    assert_eq!(shadow_slot_take(&mut slots, 10, 11, 12), (1, vec![1]));
    assert_eq!(slots.len(), 10);
}

/// Split screen: records the request of `view` for this frame and tells whether this view
/// updates the shadow, i.e. no other view saw its owner nearer during this frame or the
/// previous one (equal distances: the lowest viewport). A distance that is not a number
/// counts as infinite.
pub(crate) fn shadow_view_handles(records: &mut Vec<Option<(u32, f32)>>, view: usize, frame: u32, dist: f32) -> bool {
    let key = |d: f32| if d.is_nan() { f32::INFINITY } else { d };
    if records.len() <= view {
        records.resize(view + 1, None);
    }
    records[view] = Some((frame, dist));
    records.iter().enumerate().all(|(k, r)| match *r {
        Some((f, d)) if k != view && frame.wrapping_sub(f) <= 1 => key(dist) < key(d) || (key(dist) == key(d) && view < k),
        _ => true,
    })
}

#[cfg(test)]
#[test]
fn split_screen_shadow_is_updated_by_the_nearest_view_only() {
    // One view: every request is handled, whatever the distance.
    let mut r = Vec::new();
    for f in 1..5 {
        assert!(shadow_view_handles(&mut r, 0, f, 5000.0 * f as f32));
    }
    // Corpse near the second player (view 1), far for the first one (view 0): from the
    // second frame on, only view 1 updates it, so view 0 no longer detaches it.
    let mut r = Vec::new();
    assert!(shadow_view_handles(&mut r, 0, 10, 4000.0));
    assert!(shadow_view_handles(&mut r, 1, 10, 300.0));
    for f in 11..20 {
        assert!(!shadow_view_handles(&mut r, 0, f, 4000.0), "frame {f}");
        assert!(shadow_view_handles(&mut r, 1, f, 300.0), "frame {f}");
    }
    // The second player walks away: view 0 still sees the previous frame of view 1, then
    // takes over on the next frame.
    assert!(!shadow_view_handles(&mut r, 0, 20, 4000.0));
    assert!(!shadow_view_handles(&mut r, 1, 20, 4500.0));
    assert!(shadow_view_handles(&mut r, 0, 21, 4000.0));
    assert!(!shadow_view_handles(&mut r, 1, 21, 4500.0));
    // Corpse no longer drawn in view 0: one frame later view 1 handles it again.
    assert!(!shadow_view_handles(&mut r, 1, 22, 4500.0));
    assert!(shadow_view_handles(&mut r, 1, 23, 4500.0));
    // Same distance: the lowest viewport; NaN counts as infinite.
    let mut r = Vec::new();
    assert!(shadow_view_handles(&mut r, 0, 1, 100.0));
    assert!(!shadow_view_handles(&mut r, 1, 1, 100.0));
    assert!(shadow_view_handles(&mut r, 1, 2, f32::NAN) == false);
    assert!(shadow_view_handles(&mut r, 0, 2, 100.0));
}

/// UShadowBitmapMaterial::Update to do: silhouette of `owner` seen from
/// `location` + `light_dir` × 1000, orthographic projection of half-width `usize` / 2, in white
/// (called with 0xFF) on a black background.
#[derive(Debug, Clone)]
pub struct ShadowJob {
    pub projector: ObjId,
    /// D3DDrv target (0 to 7).
    pub slot: usize,
    pub owner: ObjId,
    pub location: V,
    pub light_dir: V,
    pub usize: i32,
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// FVector::SafeNormal (zero below 1e-8).
fn safe_normal(v: V) -> V {
    let s = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if (s as f64) < 1.0e-8 {
        return [0.0; 3];
    }
    let r = 1.0 / s.sqrt();
    [r * v[0], r * v[1], r * v[2]]
}

/// FPlane(A, B, C) = normalized normal of (B − A) × (C − A), W = N · A.
fn plane3(a: V, b: V, c: V) -> [f32; 4] {
    let (u, w) = (sub(b, a), sub(c, a));
    let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    let s = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
    // Normalized if (X² + Y²) + Z² ≥ 1e-8 (comparison in double).
    let n = if s as f64 >= 1.0e-8 { mul(n, 1.0 / s.sqrt()) } else { [0.0; 3] };
    // W = (N.X × A.X + N.Z × A.Z) + N.Y × A.Y, in the order.
    [n[0], n[1], n[2], (n[0] * a[0] + n[2] * a[2]) + n[1] * a[1]]
}

/// FCoords::Matrix (row vector: u = (p − O) · X…).
fn coords_matrix(o: V, x: V, y: V, z: V) -> [f32; 16] {
    let (nx, ny, nz) = (-o[0], -o[1], -o[2]);
    [
        x[0],
        y[0],
        z[0],
        0.0,
        x[1],
        y[1],
        z[1],
        0.0,
        x[2],
        y[2],
        z[2],
        0.0,
        (nz * x[2] + ny * x[1]) + nx * x[0],
        (nz * y[2] + ny * y[1]) + nx * y[0],
        (nz * z[2] + ny * z[1]) + nx * z[0],
        1.0,
    ]
}

/// FMatrix::TransformFVector: ((M[2]·Z + M[1]·Y) + X·M[0]) + M[3] per component.
fn transform_fvector(m: &[[f32; 4]; 4], v: V) -> V {
    [0, 1, 2].map(|k| ((m[2][k] * v[2] + m[1][k] * v[1]) + v[0] * m[0][k]) + m[3][k])
}

/// A × B (rows).
pub(crate) fn mat_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut m = [0.0; 16];
    for r in 0..4 {
        let row = &a[r * 4..r * 4 + 4];
        for c in 0..4 {
            // Sum starting from the last term.
            m[r * 4 + c] = ((b[12 + c] * row[3] + b[8 + c] * row[2]) + b[4 + c] * row[1]) + row[0] * b[c];
        }
    }
    m
}

/// FBox += point.
fn box_add(b: &mut Option<(V, V)>, p: V) {
    match b {
        Some((lo, hi)) => {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        None => *b = Some((p, p)),
    }
}

impl Vm {
    fn projector_rotation(&self, p: ObjId) -> [i32; 3] {
        self.propr(p, "Rotation").as_rotator()
    }

    /// AProjector::CalcMatrix: frustum (8 vertices, 6 planes whose inside is on the
    /// positive side), bounding box, Matrix (world → texture) and GradientMatrix (distance).
    pub(crate) fn projector_calc_matrix(&mut self, p: ObjId) {
        let texture = self.prop(p, "ProjTexture");
        if texture.as_object().is_none() && !matches!(texture, Value::Asset(_)) {
            return;
        }
        let c = coords::div_rotator(&coords::UNIT, self.projector_rotation(p));
        let (x, y, z) = (c.x, c.y, c.z);
        let l = self.propr(p, "Location").as_vector();
        let draw_scale = self.propr(p, "DrawScale").as_float();
        let (us, vs) = self.projector_texture_size(&texture);
        let u = us as f32 * draw_scale;
        let v = vs as f32 * draw_scale * self.propr(p, "VScale").as_float();
        let neg_y = [-y[0], -y[1], -y[2]];
        let a = mul(neg_y, u * 0.5);
        let b = mul(z, v * 0.5);
        let mut st = self.projectors.states.get(&p).cloned().unwrap_or_default();
        // Vertices: (B ± A) + L; PHYS_Rotating, the sum × 1.4142129 (0x3FB504EE, a
        // little less than √2): the frame rotates, the frustum encloses all its positions (A and B
        // themselves stay unchanged for what follows).
        let k = if self.propr(p, "Physics").as_byte() == 5 { f32::from_bits(0x3FB5_04EE) } else { 1.0 };
        let coin = |d: V| if k == 1.0 { add(d, l) } else { add(mul(d, k), l) };
        st.vertices[0] = coin(add(b, a));
        st.vertices[1] = coin(sub(b, a));
        st.vertices[2] = coin(sub(mul(b, -1.0), a));
        st.vertices[3] = coin(sub(a, b));
        // W: (Z + Y) + X.
        st.planes[0] = [x[0], x[1], x[2], (x[2] * l[2] + x[1] * l[1]) + x[0] * l[0]];
        let fov = self.propr(p, "FOV").as_int();
        let max_trace = self.propr(p, "MaxTraceDistance").as_int();
        if fov == 0 {
            // W: (Y + Z) + X.
            let side = |n: V, at: V| [n[0], n[1], n[2], (n[1] * at[1] + n[2] * at[2]) + n[0] * at[0]];
            let (na, nb) = (safe_normal(a), safe_normal(b));
            st.planes[1] = side(mul(nb, -1.0), st.vertices[0]);
            st.planes[2] = side(na, st.vertices[1]);
            st.planes[3] = side(nb, st.vertices[2]);
            st.planes[4] = side(mul(na, -1.0), st.vertices[3]);
            let cx = mul(y, 1.0 / u);
            let cy = mul([-z[0], -z[1], -z[2]], 1.0 / v);
            let origin = add(add(a, l), b);
            st.matrix = coords_matrix(origin, cx, cy, x);
            for k in 0..4 {
                st.vertices[k + 4] = add(mul(x, max_trace as f32), st.vertices[k]);
            }
        } else {
            let half = (fov as f32 * 0.008_726_646) as f64;
            let t = half.dtan() as f32;
            let cos = half.dcos() as f32;
            let dist = (u * 0.5) / t;
            let eye = sub(l, mul(x, dist));
            st.planes[1] = plane3(eye, st.vertices[1], st.vertices[0]);
            st.planes[2] = plane3(eye, st.vertices[2], st.vertices[1]);
            st.planes[3] = plane3(eye, st.vertices[3], st.vertices[2]);
            st.planes[4] = plane3(eye, st.vertices[0], st.vertices[3]);
            let m0 = coords_matrix(eye, y, [-z[0], -z[1], -z[2]], x);
            let mut proj = [0.0f32; 16];
            proj[0] = 1.0 / (t + t);
            let tv = t * self.propr(p, "VScale").as_float();
            proj[5] = 1.0 / (tv + tv);
            proj[8] = 0.5;
            proj[9] = 0.5;
            proj[10] = 1.0;
            proj[11] = 1.0;
            st.matrix = mat_mul(&m0, &proj);
            let s = 1.0 / cos;
            for k in 0..4 {
                let d = safe_normal(sub(st.vertices[k], eye));
                // (MaxTraceDistance × D) × (1 / cos), component by component.
                st.vertices[k + 4] = add(mul(mul(d, max_trace as f32), s), st.vertices[k]);
            }
        }
        let f = st.vertices[7];
        // W: (Y + Z) + X.
        st.planes[5] = [-x[0], -x[1], -x[2], (-x[1] * f[1] + -x[2] * f[2]) + -x[0] * f[0]];
        // GradientMatrix: V = (p − (L − X·MT/64)) · X · 62 / (64·MT).
        let back = (max_trace / 64) as f32;
        let s = 1.0 / ((max_trace << 6) as f32 * 0.016_129_032);
        let origin = sub(l, mul(x, back));
        st.gradient = coords_matrix(origin, [0.0; 3], mul(x, s), [0.0; 3]);
        st.bbox = None;
        for k in 0..8 {
            box_add(&mut st.bbox, st.vertices[k]);
        }
        if let Some(i) = st.info {
            if let Some(Some(info)) = self.projectors.infos.get_mut(i) {
                info.matrix = st.matrix;
            }
        }
        self.projectors.states.insert(p, st);
    }

    /// MaterialUSize / MaterialVSize of a ProjTexture, objects created by the script included
    /// (ShadowProjector's FinalBlend → UShadowBitmapMaterial and its USize, VSize).
    fn projector_texture_size(&mut self, texture: &Value) -> (i32, i32) {
        let mut t = texture.clone();
        for _ in 0..4 {
            match t.as_object() {
                Some(o) if self.objects.get(o).is_some_and(|x| x.is_some()) => {
                    if self.is_a(o, "FinalBlend") || self.is_a(o, "Modifier") {
                        t = self.prop(o, "Material");
                        continue;
                    }
                    return (self.propr(o, "USize").as_int(), self.propr(o, "VSize").as_int());
                }
                _ => return self.material_size(&t),
            }
        }
        (0, 0)
    }

    /// AShadowProjector::UpdateShadow, called after its owner is rendered
    /// (USkeletalMeshInstance::Render): `dist` = the LastRenderDist that
    /// FDynamicActor::Render has just set, `instance_dir` = cartoon light direction of
    /// the instance (+0x23C skeletal, +0x68 static), `frame` = the viewport's frame counter.
    /// Split screen (co-op, several viewports; the PC original draws a single view):
    /// UGameEngine::Draw runs once per viewport and each view would call UpdateShadow with
    /// its own distance. A corpse near one player and beyond ShadowMaxDist for the other was
    /// then detached by one view and attached again by the other on every frame, so every
    /// static shadow asked for a new silhouette each frame and the eight D3DDrv targets
    /// were taken from one shadow to the next (shadows blinking or missing). Here a shadow
    /// is updated once per frame, by the view where its owner is nearest (this frame or the
    /// previous one); with one view nothing changes.
    /// Split screen: viewport `view` draws frame `frame` (called for every view, with or
    /// without shadows: a view showing none still shares the targets, `shadow_slot_count`).
    pub fn shadow_view_drawn(&mut self, frame: u32, view: u8) {
        let p = &mut self.projectors;
        note_shadow_view(&mut p.view_mask, &mut p.split_frame, frame, view);
    }

    pub fn update_shadow_in_view(&mut self, sp: ObjId, dist: f32, instance_dir: Option<V>, frame: u32, view: u8) {
        let records = self.projectors.shadow_views.entry(sp).or_default();
        if shadow_view_handles(records, view as usize, frame, dist) {
            self.update_shadow(sp, dist, instance_dir, frame);
        }
    }

    /// appSeconds as the projectors read it (also inlined in AProjector::Detach): rdtsc × GSecondsPerCycle, the seconds since the processor
    /// started, so always well beyond one second while a level runs. A forced Detach sets the last
    /// render to 0, and then drops the info once `now − 0 > 1`. app_seconds counts
    /// from the launch: without an origin, an info detached during the first second was never
    /// dropped while it kept being drawn (an old shadow projection left on the ground).
    pub(crate) fn projector_now(&self) -> f64 {
        self.app_seconds + self.projectors.clock_origin
    }

    pub fn update_shadow(&mut self, sp: ObjId, dist: f32, instance_dir: Option<V>, frame: u32) {
        let Some(owner) = self.propr(sp, "Owner").as_object().filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) else {
            self.projector_detach(sp, true);
            return;
        };
        if self.propr(owner, "bHidden").as_bool() {
            self.projector_detach(sp, true);
            return;
        }
        let mode = self.level.map_or(0, |l| self.propr(l, "ShadowMode").as_int());
        if mode > 1 {
            if mode == 2 {
                self.projector_detach(sp, true);
            }
            return;
        }
        if !(dist < self.propr(sp, "ShadowMaxDist").as_float()) {
            self.projector_detach(sp, true);
            return;
        }
        let attached = self.projectors.states.get(&sp).and_then(|s| s.info).is_some();
        let texture = self.propr(sp, "ShadowTexture").as_object().filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some()));
        if !(self.propr(sp, "bShadowIsStatic").as_bool() && attached) {
            self.projector_detach(sp, true);
            let size = (self.propr(sp, "ShadowScale").as_float() * 200.0) as i32;
            if let Some(t) = texture {
                self.set_prop(t, "USize", Value::Int(size));
                self.set_prop(t, "VSize", Value::Int(size));
            }
            let l = self.prop(owner, "Location");
            self.set_prop(sp, "Location", l);
            let dir = shadow_light_dir(self.propr(owner, "DrawType").as_byte(), instance_dir);
            self.set_prop(sp, "LightDirection", Value::vector(dir[0], dir[1], dir[2]));
            let r = crate::physics::vector_rotation([-dir[0], -dir[1], -dir[2]]);
            self.set_prop(sp, "Rotation", Value::rotator(r[0], r[1], r[2]));
        }
        let dir = self.propr(sp, "LightDirection").as_vector();
        if let Some(t) = texture {
            self.set_prop(t, "LightDirection", Value::vector(dir[0], dir[1], dir[2]));
        }
        let fade = shadow_fade(dist, self.propr(sp, "ShadowMaxDist").as_float(), self.propr(sp, "ShadowTransDist").as_float());
        let intensity = (self.propr(sp, "ShadowIntensity").as_int() as f32 * fade) as i32;
        self.projectors.shadow_intensity.insert(sp, intensity);
        // _ftol pops the product, the test is on the attenuation factor alone
        // (a shadow of intensity 0 is still attached and drawn).
        if fade == 0.0 {
            return;
        }
        let f = if mode == 0 { frame << 1 } else { frame };
        let schedule = if self.projectors.states.get(&sp).and_then(|s| s.info).is_none() {
            self.projector_attach(sp);
            let now = self.projector_now();
            if let Some(i) = self.projectors.states.get(&sp).and_then(|s| s.info) {
                if let Some(Some(info)) = self.projectors.infos.get_mut(i) {
                    info.expiry = now + 10.0;
                }
            }
            true
        } else {
            !self.projectors.shadow_valid.contains(&sp)
        };
        if !schedule {
            return;
        }
        let count = self.propr(sp, "UpdateCount").as_int() as u32;
        let job = shadow_schedule(&mut self.projectors.shadow_counter, count, f).is_some();
        if job {
            let c = self.projectors.shadow_counter;
            self.set_prop(sp, "UpdateCount", Value::Int(c as i32));
            let usize = texture.map_or(0, |t| self.propr(t, "USize").as_int());
            let location = self.propr(owner, "Location").as_vector();
            self.projectors.shadow_stamp += 1;
            let stamp = self.projectors.shadow_stamp;
            let used = shadow_slot_count(self.projectors.split_frame.is_some_and(|l| frame.wrapping_sub(l) <= 1), self.projectors.more_shadow_targets);
            let (slot, lost) = shadow_slot_take(&mut self.projectors.shadow_slots, used, sp, stamp);
            for old in lost {
                self.projectors.shadow_valid.remove(&old);
            }
            self.projectors.shadow_jobs.push(ShadowJob {
                projector: sp,
                slot,
                owner,
                location,
                light_dir: dir,
                usize,
            });
            self.projectors.shadow_valid.insert(sp);
        }
    }

    /// The renderer made its shadow targets again (new size): every shadow is drawn again at
    /// its next update, as if its target had been taken by another.
    pub fn invalidate_shadow_targets(&mut self) {
        self.projectors.shadow_valid.clear();
    }

    fn info_release(&mut self, i: usize) {
        if let Some(slot) = self.projectors.infos.get_mut(i) {
            if let Some(info) = slot {
                info.refs -= 1;
                if info.refs == 0 {
                    *slot = None;
                }
            }
        }
    }

    fn info_add_ref(&mut self, i: usize) {
        if let Some(Some(info)) = self.projectors.infos.get_mut(i) {
            info.refs += 1;
        }
    }

    /// AProjector::Attach.
    pub(crate) fn projector_attach(&mut self, p: ObjId) {
        let calc_replaces = false;
        if !calc_replaces {
            self.projector_calc_matrix(p);
        }
        let st = self.projectors.states.get(&p).cloned().unwrap_or_default();
        // FProjectorRenderInfo without an end of life, one reference for the projector.
        let flag = |vm: &Vm, n: &str| vm.propr(p, n).as_bool();
        let mut flags =
            (((flag(self, "bProjectOnAlpha") as u16) << 2 | flag(self, "bProjectOnUnlit") as u16) << 1) | (self.propr(p, "FOV").as_int() != 0) as u16;
        if flag(self, "bFade") {
            flags |= 4;
        }
        let info = ProjInfo {
            geometry_life: std::rc::Rc::new(()),
            refs: 1,
            last_render: 0.0,
            expiry: 0.0,
            material: self.prop(p, "ProjTexture"),
            matrix: st.matrix,
            gradient: st.gradient,
            flags,
            priority: self.propr(p, "AttachPriority").as_byte(),
            projector: p,
        };
        let index = match self.projectors.infos.iter().position(Option::is_none) {
            Some(i) => {
                self.projectors.infos[i] = Some(info);
                i
            }
            None => {
                self.projectors.infos.push(Some(info));
                self.projectors.infos.len() - 1
            }
        };
        if let Some(s) = self.projectors.states.get_mut(&p) {
            s.info = Some(index);
        }
        // bProjectTerrain: sectors of the zone's terrains (ZoneInfo.Terrains) that the box touches.
        if flag(self, "bProjectTerrain") {
            if let Some((lo, hi)) = st.bbox {
                self.projector_attach_terrains(p, index, lo, hi);
            }
        }
        if flag(self, "bProjectBSP") {
            if let Some((lo, hi)) = st.bbox {
                let dir = coords::div_rotator(&coords::UNIT, self.projector_rotation(p)).x;
                let nodes = self.convex_volume_multi_check(lo, hi, &st.planes, dir);
                let clip = flag(self, "bClipBSP");
                for n in nodes {
                    self.info_add_ref(index);
                    self.model_attach_projector(n, index, clip.then_some(&st.planes));
                }
            }
        }
        // bProjectStaticMesh / bProjectActor: actors of the collision hash whose box touches
        // the projector's (vfunc + 0x18, FCollisionOctree::ActorPointCheckOnlyForProjectors,
        // centre and half-sizes of Box), that accept projectors
        // (bAcceptsProjectors, sign bit of + 0x2C), with ProjectTag if it is set, with a
        // StaticMesh (+ 0x13C, bit 4) or a Mesh (+ 0x138, bit 8) depending on the flag.
        let (sm, actor) = (flag(self, "bProjectStaticMesh"), flag(self, "bProjectActor"));
        if sm || actor {
            if let Some((lo, hi)) = st.bbox {
                let tag = self.propr(p, "ProjectTag").as_name();
                let tagged = !tag.is_empty() && !tag.eq_ignore_ascii_case("None");
                let center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
                let extent = [(hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5, (hi[2] - lo[2]) * 0.5];
                let candidates: Vec<ObjId> = self
                    .octree_projector_check(center, extent)
                    .into_iter()
                    .filter(|&a| self.alive(a) && self.pb(a, "bAcceptsProjectors"))
                    .filter(|&a| !tagged || self.propr(a, "Tag").as_name().eq_ignore_ascii_case(&tag))
                    .filter(|&a| {
                        let set = |v: Value| matches!(v, Value::Asset(ref m) if !m.is_empty()) || matches!(v, Value::Object(Some(_)));
                        (sm && set(self.prop(a, "StaticMesh"))) || (actor && set(self.prop(a, "Mesh")))
                    })
                    .collect();
                for a in candidates {
                    let (blo, bhi) = self.render_bounding_box(a, true);
                    if (0..3).any(|k| blo[k] > hi[k] || lo[k] > bhi[k]) {
                        continue;
                    }
                    self.actor_attach_projector(a, p, index, &st.planes);
                }
            }
        }
    }

    /// UProjectorPrimitive::PointCheck: true (contact) if the box `location` ±
    /// `extent` is outside none of the six planes of FrustumPlanes: outside a plane when
    /// ((Y·Ny + Z·Nz) + X·Nx) − W < −((|Ez·Nz| + |Ey·Ny|) + |Ex·Nx|).
    pub(crate) fn projector_point_check(&self, p: ObjId, location: V, extent: V) -> bool {
        let planes = self.projectors.states.get(&p).map_or([[0.0; 4]; 6], |s| s.planes);
        projector_point_core(&planes, location, extent)
    }

    /// StaticMesh+0x184 of a StaticMesh (openxiii_assets::static_collision), read once; 0 if
    /// the mesh is missing.
    fn projector_raw_triangles(&mut self, path: &str) -> i16 {
        if let Some(&n) = self.projectors.raw_triangles.get(path) {
            return n;
        }
        let n = self
            .library
            .as_mut()
            .and_then(|lib| {
                let (loaded, index) = lib.resolve(path)?;
                if loaded.package.export_class(index) != "StaticMesh" {
                    return None;
                }
                openxiii_assets::static_collision::read(&loaded.package, &loaded.data, index).ok()
            })
            .map_or(0, |c| c.raw_triangles);
        self.projectors.raw_triangles.insert(path.to_string(), n);
        n
    }

    /// StaticMesh of a path, read once.
    fn projector_mesh(&mut self, path: &str) -> Option<std::rc::Rc<openxiii_assets::static_mesh::StaticMesh>> {
        if let Some(m) = self.projectors.meshes.get(path) {
            return m.clone();
        }
        let mesh = self.library.as_mut().and_then(|lib| {
            let (loaded, index) = lib.resolve(path)?;
            if loaded.package.export_class(index) != "StaticMesh" {
                return None;
            }
            let mut mesh = openxiii_assets::static_mesh::StaticMesh::read(&loaded.package, &loaded.data, index).ok()?;
            // FStaticMeshMaterial flags read by UStaticMesh::Serialize: bit 0 =
            // first byte of the file, bit 1 = second. Before licensee version 0x38 bit 1 is set
            // on all of them (empty list: no filter).
            if let Ok(c) = openxiii_assets::static_collision::read(&loaded.package, &loaded.data, index) {
                if !c.native_materials.is_empty() {
                    mesh.material_flags = c.native_materials.iter().map(|m| m.1).collect();
                }
            }
            Some(std::rc::Rc::new(mesh))
        });
        self.projectors.meshes.insert(path.to_string(), mesh.clone());
        mesh
    }

    /// LocalToWorld of the actor (Rotation, DrawScale3D, Location, PrePivot,
    /// DrawScale), like AActor::AttachProjector and the drawing of the projectors.
    fn projector_local_to_world(&self, a: ObjId) -> [[f32; 4]; 4] {
        let s3 = match self.propr(a, "DrawScale3D") {
            v if v.is_struct() => v.as_vector(),
            _ => [1.0; 3],
        };
        openxiii_assets::lighting::local_to_world(
            self.propr(a, "Rotation").as_rotator(),
            s3,
            self.propr(a, "DrawScale").as_float(),
            self.propr(a, "Location").as_vector(),
            self.propr(a, "PrePivot").as_vector(),
        )
    }

    /// AActor::AttachProjector: at most 10 entries, minus one per 100 of
    /// StaticMesh+0x184 (no attachment if this number is not positive), only one per info; the lowest priority one,
    /// then the one closest to its end of life, is replaced. A StaticMesh attached by a
    /// bStatic or non-bMovable projector keeps the triangles (of all the sections: bit 2
    /// of the materials is always set) that no plane of the frustum has entirely on the negative side; without a triangle, the entry
    /// is removed.
    fn actor_attach_projector(&mut self, a: ObjId, p: ObjId, info: usize, planes: &[[f32; 4]; 6]) {
        let max = match self.prop(a, "StaticMesh") {
            Value::Asset(m) if !m.is_empty() => 10 - self.projector_raw_triangles(&m) as i32 / 100,
            _ => 10,
        };
        if max <= 0 {
            return;
        }
        let max = max as usize;
        let list = self.projectors.actors.entry(a).or_default();
        if list.iter().any(|e| e.info == info) {
            return;
        }
        let slot = if list.len() < max {
            list.push(ActorProjector { info, indices: None, indexed_poly: None });
            self.info_add_ref(info);
            self.projectors.actors[&a].len() - 1
        } else {
            let k = |i: usize| self.projectors.infos.get(i).and_then(Option::as_ref).map_or((0, 0.0), |x| (x.priority, x.expiry));
            let list = &self.projectors.actors[&a];
            let (mut best, (mut bp, mut be)) = (0, k(list[0].info));
            for (n, e) in list.iter().enumerate().skip(1) {
                let (pr, ex) = k(e.info);
                if pr < bp || (pr == bp && ex < be) {
                    best = n;
                    bp = pr;
                    be = ex;
                }
            }
            let old = list[best].info;
            self.info_release(old);
            self.info_add_ref(info);
            self.projectors.actors.get_mut(&a).unwrap()[best] = ActorProjector { info, indices: None, indexed_poly: None };
            best
        };
        let fixed = self.native_prop(p, crate::NativeField::Static).as_bool() || !self.pb(p, "bMovable");
        let mesh_path = match self.prop(a, "StaticMesh") {
            Value::Asset(m) if !m.is_empty() => Some(m),
            _ => None,
        };
        let (Some(path), true) = (mesh_path, fixed) else { return };
        let Some(mesh) = self.projector_mesh(&path) else { return };
        let l2w = self.projector_local_to_world(a);
        let world: Vec<V> = mesh.positions.iter().map(|&v| transform_fvector(&l2w, v)).collect();
        // Signed distance of each vertex to the plane, in the order of the asm's sums: list
        // (Y+Z)+X for the first vertex, (X+Y)+Z for the other two; strip
        // (Z+X)+Y for the first, (X+Z)+Y for the other two. Outside the plane if the
        // three are < 0 (NaN: inside).
        let outside = |t: [u16; 3], strip: bool| -> bool {
            let w = |i: u16| world.get(i as usize).copied();
            let (Some(a), Some(b), Some(c)) = (w(t[0]), w(t[1]), w(t[2])) else { return false };
            planes.iter().any(|pl| {
                let (da, db, dc) = if strip {
                    (
                        (a[2] * pl[2] + a[0] * pl[0]) + a[1] * pl[1],
                        (b[0] * pl[0] + b[2] * pl[2]) + b[1] * pl[1],
                        (c[0] * pl[0] + c[2] * pl[2]) + c[1] * pl[1],
                    )
                } else {
                    (
                        (a[1] * pl[1] + a[2] * pl[2]) + a[0] * pl[0],
                        (b[0] * pl[0] + b[1] * pl[1]) + b[2] * pl[2],
                        (c[0] * pl[0] + c[1] * pl[1]) + c[2] * pl[2],
                    )
                };
                da - pl[3] < 0.0 && db - pl[3] < 0.0 && dc - pl[3] < 0.0
            })
        };
        let mut indices: Vec<u16> = Vec::new();
        for (si, sec) in mesh.sections.iter().enumerate() {
            // AActor::AttachProjector: `test byte [Materials + section·8 + 4], 2`, bit
            // 1 of the section's material (second byte of the file; 1812 meshes out of 6128
            // have materials without this bit). Without flags read, all are projectable.
            if mesh.material_flags.get(si).is_some_and(|f| f & 2 == 0) {
                continue;
            }
            if sec.primitives == 0 {
                continue;
            }
            let first = sec.first_index as usize;
            let n = sec.primitives as usize;
            if sec.is_strip == 0 {
                for k in 0..n {
                    let Some(t) = mesh.indices.get(first + 3 * k..first + 3 * k + 3) else { break };
                    let t = [t[0], t[1], t[2]];
                    if !outside(t, false) {
                        indices.extend(t);
                    }
                }
            } else {
                // Strip: NumPrimitives − 2 triangles (bounds of the original code), without inversion.
                let (Some(&i0), Some(&i1)) = (mesh.indices.get(first), mesh.indices.get(first + 1)) else { continue };
                let (mut u4, mut u5) = (i0, i1);
                for j in 2..n {
                    let Some(&u3) = mesh.indices.get(first + j) else { break };
                    let t = [u4, u5, u3];
                    if !outside(t, true) {
                        indices.extend(t);
                    }
                    u4 = u5;
                    u5 = u3;
                }
            }
        }
        let list = self.projectors.actors.get_mut(&a).unwrap();
        if indices.is_empty() {
            // The entry is removed, its destructor gives back the reference.
            let removed = list.remove(slot);
            self.info_release(removed.info);
        } else {
            list[slot].indices = Some(indices);
        }
    }

    /// Projectors of the drawn StaticMeshes: stale entries removed, triangles
    /// of the entry (or of all the sections) in world coordinates, without
    /// lighting or SetZBias.
    pub fn actor_projector_draws(&mut self, drawn: &std::collections::HashSet<ObjId>) -> Vec<ProjectorDraw> {
        let now = self.projector_now();
        let mut out = Vec::new();
        let mut keys = std::mem::take(&mut self.projectors.draw_actors);
        keys.clear();
        keys.extend(self.projectors.actors.keys().copied());
        for &a in &keys {
            if !self.alive(a) {
                self.projectors.actors.remove(&a);
                self.projectors.actor_polys.remove(&a);
                continue;
            }
            if !drawn.contains(&a) {
                continue;
            }
            let mesh_name = match self.prop(a, "StaticMesh") {
                Value::Asset(m) if !m.is_empty() => m,
                _ => continue,
            };
            let Some(mesh) = self.projector_mesh(&mesh_name) else { continue };
            let l2w = self.projector_local_to_world(a);
            let bits = l2w.map(|r| r.map(f32::to_bits));
            let mut k = 0;
            while let Some(info_index) = self.projectors.actors.get(&a).and_then(|l| l.get(k)).map(|e| e.info) {
                let valid = self.projectors.infos.get_mut(info_index).and_then(Option::as_mut).is_some_and(|info| info.still_valid(now));
                if !valid {
                    self.projectors.actors.get_mut(&a).unwrap().remove(k);
                    self.info_release(info_index);
                    continue;
                }
                let info = self.projectors.infos[info_index].clone().unwrap();
                let shadow = self.alive(info.projector) && self.is_a(info.projector, "ShadowProjector");
                if shadow && !self.projectors.shadow_valid.contains(&info.projector) {
                    k += 1;
                    continue;
                }
                let entry = &mut self.projectors.actors.get_mut(&a).unwrap()[k];
                let poly: Rc<[V]> = match &entry.indices {
                    Some(indices) => match &entry.indexed_poly {
                        Some((cached_mesh, transform, poly)) if Rc::ptr_eq(cached_mesh, &mesh) && *transform == bits => poly.clone(),
                        _ => {
                            let poly: Rc<[V]> = indices.iter().filter_map(|&i| mesh.positions.get(i as usize)).map(|&v| transform_fvector(&l2w, v)).collect();
                            entry.indexed_poly = Some((mesh.clone(), bits, poly.clone()));
                            poly
                        }
                    },
                    None => {
                        match self.projectors.actor_polys.get(&a).filter(|(m, b, _)| *m == mesh_name && *b == bits) {
                            Some((_, _, p)) => p.clone(),
                            None => {
                                let p: std::rc::Rc<[V]> = sections_projectable(&mesh)
                                    .into_iter()
                                    .flat_map(|si| mesh.section_triangles(&mesh.sections[si]))
                                    .flatten()
                                    .filter_map(|i| mesh.positions.get(i as usize))
                                    .map(|&v| transform_fvector(&l2w, v))
                                    .collect();
                                self.projectors.actor_polys.insert(a, (mesh_name.clone(), bits, p.clone()));
                                p
                            }
                        }
                    }
                };
                let slot = self.projectors.shadow_slots.iter().position(|s| s.is_some_and(|(p, _)| p == info.projector)).unwrap_or(0);
                let texture = if shadow { format!("#shadow:{slot}:{}", self.projectors.shadow_intensity.get(&info.projector).copied().unwrap_or(0)) } else { crate::canvas::material_path(&info.material) };
                out.push(ProjectorDraw { fragment: [1, a, 0], node: a, poly, triangles: true, texture, info });
                k += 1;
            }
            if self.projectors.actors.get(&a).is_some_and(|l| l.is_empty()) {
                self.projectors.actors.remove(&a);
                self.projectors.actor_polys.remove(&a);
            }
        }
        self.projectors.draw_actors = keys;
        out
    }

    /// Terrains of the map: native data (ATerrainInfo::Serialize) and EdgeTurnBitmap.
    fn terrain_cache(&mut self) {
        if self.projectors.terrains.is_some() {
            return;
        }
        let mut out = std::collections::HashMap::new();
        if let Some(pi) = self.map_package {
            let actors: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.alive(a) && self.is_a(a, "TerrainInfo")).collect();
            for a in actors {
                let name = self.objects[a].as_ref().map(|o| o.name.to_lowercase()).unwrap_or_default();
                let data = {
                    let lp = &self.packages[pi];
                    (0..lp.package.exports.len())
                        .find(|&i| {
                            lp.package.export_class(i) == "TerrainInfo"
                                && lp.package.object_name(openxiii_package::ObjectRef::Export(i)).eq_ignore_ascii_case(&name)
                        })
                        .and_then(|i| openxiii_assets::terrain::read_terrain(&lp.package, &lp.data, i).ok())
                };
                let Some(data) = data else { continue };
                // ATerrainInfo::PostLoad: bitmap at the size of the heightmap, otherwise zero.
                let words = (data.heightmap_x.max(0) as usize * data.heightmap_y.max(0) as usize) / 32;
                let mut edge_turn: Vec<u32> = match self.prop(a, "EdgeTurnBitmap") {
                    Value::Array(v) => v.iter().map(|x| x.as_int() as u32).collect(),
                    _ => Vec::new(),
                };
                if edge_turn.len() != words {
                    edge_turn = vec![0; words];
                }
                out.insert(a, TerrainCache { data, edge_turn });
            }
        }
        self.projectors.terrains = Some(out);
    }

    /// Terrain part of AProjector::Attach: for each terrain of ZoneInfo.Terrains,
    /// rectangle of sectors covered by the box (1 / (TerrainScale × 8) per unit, grid centred
    /// at Sectors / 2, _ftol truncations and + 0.99 at the upper bounds), abandoned if it leaves
    /// the grid, clamped otherwise, then UTerrainSector::AttachProjector for each sector.
    fn projector_attach_terrains(&mut self, p: ObjId, info: usize, lo: V, hi: V) {
        let zone = match self.prop(p, "Region") {
            Value::Struct(_, f) => f.first().and_then(|z| z.as_object()),
            _ => None,
        };
        let Some(zone) = zone.filter(|&z| self.alive(z)) else { return };
        let terrains: Vec<ObjId> = match self.prop(zone, "Terrains") {
            Value::Array(v) => v.iter().filter_map(|x| x.as_object()).collect(),
            _ => Vec::new(),
        };
        if terrains.is_empty() {
            return;
        }
        self.terrain_cache();
        for t in terrains {
            let Some((sx, sy)) = self.projectors.terrains.as_ref().and_then(|c| c.get(&t)).map(|c| (c.data.sectors_x, c.data.sectors_y)) else { continue };
            let scale = self.propr(t, "TerrainScale").as_vector();
            let loc = self.propr(t, "Location").as_vector();
            let inv_x = 1.0 / (scale[0] * 8.0);
            let a = (lo[0] - loc[0]) * inv_x;
            let b = (hi[0] - loc[0]) * inv_x;
            let inv_y = 1.0 / (scale[1] * 8.0);
            let c = (lo[1] - loc[1]) * inv_y;
            let d = (hi[1] - loc[1]) * inv_y;
            let (hx, hy) = ((sx / 2) as f32, (sy / 2) as f32);
            let mut x0 = (hx + a) as i32;
            let mut x1 = ((hx + b) + 0.99) as i32;
            let mut y0 = (hy + c) as i32;
            let mut y1 = ((hy + d) + 0.99) as i32;
            if x1 < 0 || y1 < 0 || x0 >= sx || y0 >= sy {
                continue;
            }
            x0 = x0.max(0);
            y0 = y0.max(0);
            if x1 >= sx {
                x1 = sx - 1;
            }
            if y1 >= sy {
                y1 = sy - 1;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    self.sector_attach_projector(t, (sx * y + x) as usize, info, lo, hi, scale);
                }
            }
        }
    }

    /// UTerrainSector::AttachProjector: if the sector's box touches the
    /// projector's, an entry (at most 10, the lowest priority then the one closest to its end of
    /// life replaced) with the two triangles of each quad of the covered rectangle (EdgeTurnBitmap
    /// chooses the diagonal); the entry is removed if it has no triangle.
    fn sector_attach_projector(&mut self, t: ObjId, s: usize, info: usize, lo: V, hi: V, scale: V) {
        let Some(cache) = self.projectors.terrains.as_ref().and_then(|c| c.get(&t)) else { return };
        let Some(sector) = cache.data.sectors.get(s) else { return };
        let (mut blo, mut bhi) = (sector.bounds[0], sector.bounds[0]);
        for p in &sector.bounds[1..] {
            for k in 0..3 {
                blo[k] = blo[k].min(p[k]);
                bhi[k] = bhi[k].max(p[k]);
            }
        }
        // The sector's box widened by the projector's half-size must contain
        // its centre (FBox::GetCenter), bounds included; (point in a box,
        // a NaN never is).
        let center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
        let ext = [(hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5, (hi[2] - lo[2]) * 0.5];
        if !(0..3).all(|k| blo[k] - ext[k] <= center[k] && center[k] <= bhi[k] + ext[k]) {
            return;
        }
        let plo = [center[0] - ext[0], center[1] - ext[1], center[2] - ext[2]];
        let phi = [center[0] + ext[0], center[1] + ext[1], center[2] + ext[2]];
        let (qx, qy, ox, oy) = (sector.quads_x, sector.quads_y, sector.offset_x, sector.offset_y);
        let hm_x = cache.data.heightmap_x;
        let origin = cache.data.vertices.get((hm_x * oy + ox) as usize).copied().unwrap_or([0.0; 3]);
        self.info_add_ref(info);
        let key = (t, s);
        let list = self.projectors.sectors.entry(key).or_default();
        let slot = if list.len() < 10 {
            list.push(SectorProjector { info, indices: Vec::new(), poly: Default::default() });
            list.len() - 1
        } else {
            let k = |i: usize| self.projectors.infos.get(i).and_then(Option::as_ref).map_or((0, 0.0), |x| (x.priority, x.expiry));
            let list = &self.projectors.sectors[&key];
            let (mut best, (mut bp, mut be)) = (0, k(list[0].info));
            for (n, e) in list.iter().enumerate().skip(1) {
                let (p, ex) = k(e.info);
                if p < bp || (p == bp && ex < be) {
                    best = n;
                    bp = p;
                    be = ex;
                }
            }
            let old = list[best].info;
            self.info_release(old);
            let list = self.projectors.sectors.get_mut(&key).unwrap();
            list[best] = SectorProjector { info, indices: Vec::new(), poly: Default::default() };
            best
        };
        // Rectangle of quads: (box − first vertex of the sector) / TerrainScale, truncated, + 0.999
        // at the upper bounds.
        let (mnx, mny) = (plo[0] - origin[0], plo[1] - origin[1]);
        let (mxx, mxy) = (phi[0] - origin[0], phi[1] - origin[1]);
        let mut x0 = (mnx / scale[0]) as i32;
        let mut y0 = (mny / scale[1]) as i32;
        let mut x1 = (mxx / scale[0] + 0.999) as i32;
        let mut y1 = (mxy / scale[1] + 0.999) as i32;
        x0 = x0.max(0);
        y0 = y0.max(0);
        if x1 >= qx {
            x1 = qx - 1;
        }
        if y1 >= qy {
            y1 = qy - 1;
        }
        let edge_turn = &self.projectors.terrains.as_ref().unwrap()[&t].edge_turn;
        let mut indices: Vec<u16> = Vec::new();
        for x in x0..=x1 {
            for y in y0..=y1 {
                let i = (qx + 1) * y + x;
                let v2 = (y + 1) * (qx + 1) + 1 + x;
                let bit = (oy + y) * hm_x + ox + x;
                let turned = edge_turn.get((bit >> 5) as usize).is_some_and(|w| w & (1u32 << (bit & 31)) != 0);
                let (a, b, c, d) = (i as u16, (v2 - 1) as u16, v2 as u16, (i + 1) as u16);
                if turned {
                    indices.extend([a, b, d, b, c, d]);
                } else {
                    indices.extend([a, b, c, a, c, d]);
                }
            }
        }
        let list = self.projectors.sectors.get_mut(&key).unwrap();
        if indices.is_empty() {
            // The entry is removed, its destructor gives back the reference.
            list.remove(slot);
            self.info_release(info);
        } else {
            list[slot] = SectorProjector { info, indices, poly: Default::default() };
        }
    }

    /// Projectors of the terrain sectors (ATerrainInfo::Render): stale entries
    /// removed, triangles of each valid entry in world coordinates.
    pub fn terrain_projector_draws(&mut self) -> Vec<ProjectorDraw> {
        let now = self.projector_now();
        let mut out = Vec::new();
        let mut keys = std::mem::take(&mut self.projectors.draw_sectors);
        keys.clear();
        keys.extend(self.projectors.sectors.keys().copied());
        for &key in &keys {
            let mut k = 0;
            while let Some(info_index) = self.projectors.sectors.get(&key).and_then(|l| l.get(k)).map(|e| e.info) {
                let valid = self.projectors.infos.get_mut(info_index).and_then(Option::as_mut).is_some_and(|info| info.still_valid(now));
                if !valid {
                    self.projectors.sectors.get_mut(&key).unwrap().remove(k);
                    self.info_release(info_index);
                    continue;
                }
                let info = self.projectors.infos[info_index].clone().unwrap();
                let shadow = self.alive(info.projector) && self.is_a(info.projector, "ShadowProjector");
                if shadow && !self.projectors.shadow_valid.contains(&info.projector) {
                    k += 1;
                    continue;
                }
                let Some(cache) = self.projectors.terrains.as_ref().and_then(|c| c.get(&key.0)) else { break };
                let Some(sector) = cache.data.sectors.get(key.1) else { break };
                let (w, h) = (cache.data.heightmap_x, cache.data.heightmap_y);
                let qx = sector.quads_x;
                let vertex = |i: u16| {
                    let (x, y) = (i as i32 % (qx + 1), i as i32 / (qx + 1));
                    let (gx, gy) = ((sector.offset_x + x).min(w - 1), (sector.offset_y + y).min(h - 1));
                    cache.data.vertices.get((gy * w + gx) as usize).copied().unwrap_or([0.0; 3])
                };
                let entry = &self.projectors.sectors[&key][k];
                let poly = entry.poly.get_or_init(|| entry.indices.iter().map(|&i| vertex(i)).collect()).clone();
                let slot = self.projectors.shadow_slots.iter().position(|s| s.is_some_and(|(p, _)| p == info.projector)).unwrap_or(0);
                let texture = if shadow { format!("#shadow:{slot}:{}", self.projectors.shadow_intensity.get(&info.projector).copied().unwrap_or(0)) } else { crate::canvas::material_path(&info.material) };
                out.push(ProjectorDraw { fragment: [2, key.0, key.1], node: key.1, poly, triangles: true, texture, info });
                k += 1;
            }
            if self.projectors.sectors.get(&key).is_some_and(|l| l.is_empty()) {
                self.projectors.sectors.remove(&key);
            }
        }
        self.projectors.draw_sectors = keys;
        out
    }

    /// AProjector::Detach: end of life now; `force` forgets the last render.
    pub(crate) fn projector_detach(&mut self, p: ObjId, force: bool) {
        let Some(i) = self.projectors.states.get(&p).and_then(|s| s.info) else { return };
        let now = self.projector_now();
        if let Some(Some(info)) = self.projectors.infos.get_mut(i) {
            info.expiry = now;
            if force {
                info.last_render = 0.0;
            }
        }
        self.info_release(i);
        if let Some(s) = self.projectors.states.get_mut(&p) {
            s.info = None;
        }
    }

    /// AProjector::PostEditChange, after AActor::PostEditChange
    /// run by the dispatch: forced detach then the attachment is rebuilt.
    pub(crate) fn projector_post_edit_change(&mut self, p: ObjId) {
        self.projector_detach(p, true);
        self.projector_attach(p);
    }

    /// AProjector::execAbandonProjector: the duration passed is overwritten by 1.0 before
    /// use (original code); end of life in one second, then AProjector::Abandon.
    pub(crate) fn projector_abandon(&mut self, p: ObjId) {
        let Some(i) = self.projectors.states.get(&p).and_then(|s| s.info) else { return };
        let now = self.projector_now();
        if let Some(Some(info)) = self.projectors.infos.get_mut(i) {
            info.expiry = now + 1.0f32 as f64;
        }
        self.info_release(i);
        if let Some(s) = self.projectors.states.get_mut(&p) {
            s.info = None;
        }
    }

    /// AProjector::Destroy: Detach(1). AShadowProjector::Destroy: Detach(1), then `delete` (vtable +0x10,
    /// flag 1) of ShadowMaterial (+0x388) and of ShadowTexture (+0x384) if they are not null;
    /// the properties keep their value.
    pub(crate) fn projector_destroyed(&mut self, p: ObjId) {
        if self.projectors.states.contains_key(&p) {
            self.projector_detach(p, true);
            self.projectors.states.remove(&p);
        }
        self.projectors.shadow_views.remove(&p);
        if self.is_a(p, "ShadowProjector") {
            for name_value in ["ShadowMaterial", "ShadowTexture"] {
                if let Some(o) = self.propr(p, name_value).as_object() {
                    self.removes_object(o);
                }
            }
        }
    }

    /// AActor::Destroy: as long as Actor.Projectors (+0x128) is not empty, Remove(Num − 1, 1), whose element
    /// destructor gives back the render info (reference removed, freed at zero) and frees the buffer of the triangles touched.
    pub(crate) fn actor_projectors_destroyed(&mut self, a: ObjId) {
        if let Some(mut list) = self.projectors.actors.remove(&a) {
            while let Some(e) = list.pop() {
                self.info_release(e.info);
            }
        }
    }

    /// `delete` of a UObject (~UObject: removed from GObjObjects and the name table): the slot becomes empty, the references that
    /// remain elsewhere no longer designate anything.
    pub(crate) fn removes_object(&mut self, o: ObjId) {
        let Some(x) = self.objects.get(o).and_then(|x| x.as_ref()) else { return };
        let path = format!("{}.{}", x.outer, x.name).to_lowercase();
        self.object_paths.remove(&path);
        self.objects[o] = None;
    }

    /// UModel::ConvexVolumeMultiCheck: BSP nodes facing `dir` whose
    /// box touches the volume's (half-size × 1.1), then those for which no plane
    /// has all the vertices on the negative side (except PF_Invisible surfaces, kept).
    fn convex_volume_multi_check(&self, lo: V, hi: V, planes: &[[f32; 4]; 6], dir: V) -> Vec<usize> {
        let Some(bsp) = self.level_bsp.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        if bsp.nodes.is_empty() {
            return out;
        }
        let center = [(hi[0] + lo[0]) * 0.5, (hi[1] + lo[1]) * 0.5, (hi[2] + lo[2]) * 0.5];
        let extent = [(hi[0] - lo[0]) * 0.5 * 1.1, (hi[1] - lo[1]) * 0.5 * 1.1, (hi[2] - lo[2]) * 0.5 * 1.1];
        fn walk(bsp: &openxiii_assets::bsp::BspModel, mut n: i32, center: V, extent: V, dir: V, out: &mut Vec<usize>) {
            while n != -1 {
                let node = &bsp.nodes[n as usize];
                let p = node.plane;
                // (FBoxPushOut): radius of the box along the normal.
                let r = (p[2] * extent[2]).abs() + (p[1] * extent[1]).abs() + (p[0] * extent[0]).abs();
                // (Y + Z) + X − W on the 24-bit FPU.
                let d = (((center[1] * p[1] + center[2] * p[2]) + center[0] * p[0]) - p[3]) as f64;
                let r = r as f64;
                if d <= r {
                    if -r <= d {
                        // (Z + Y) + X compared with the double −0.0001.
                        if -0.0001f64 <= ((dir[2] * p[2] + dir[1] * p[1]) + dir[0] * p[0]) as f64 {
                            if node.coplanar != -1 {
                                walk(bsp, node.coplanar as i32, center, extent, dir, out);
                            }
                        } else {
                            let mut c = n;
                            while c != -1 {
                                out.push(c as usize);
                                c = bsp.nodes[c as usize].coplanar as i32;
                            }
                        }
                        walk(bsp, node.child[1] as i32, center, extent, dir, out);
                        n = node.child[0] as i32;
                    } else {
                        n = node.child[0] as i32;
                    }
                } else {
                    n = node.child[1] as i32;
                }
            }
        }
        walk(bsp, 0, center, extent, dir, &mut out);
        let mut i = 0;
        while i < out.len() {
            let node = &bsp.nodes[out[i]];
            let invisible = bsp.surfs.get(node.surf as usize).is_some_and(|s| s.poly_flags & 1 != 0);
            let mut removed = false;
            if !invisible {
                'planes: for pl in planes {
                    for k in 0..node.num_vertices as usize {
                        let Some(&vi) = bsp.verts.get(node.vert_pool as usize + k) else { continue };
                        let v = bsp.points[vi as usize];
                        // (Z + Y) + X.
                        if 0.0 < ((v[2] * pl[2] + v[1] * pl[1]) + pl[0] * v[0]) - pl[3] {
                            continue 'planes;
                        }
                    }
                    removed = true;
                    break;
                }
            }
            if removed {
                out.remove(i);
            } else {
                i += 1;
            }
        }
        out
    }

    /// UModel::AttachProjector: at most 10 projectors per node (the lowest priority,
    /// then the closest end of life, is replaced); with `clip`, the node's polygon (vertices
    /// in reverse order) clipped by the six planes, the entry removed if nothing is left.
    fn model_attach_projector(&mut self, node: usize, info: usize, clip: Option<&[[f32; 4]; 6]>) {
        let list = self.projectors.nodes.entry(node).or_default();
        let slot = if list.len() < 10 {
            list.push(NodeProjector { info, poly: Rc::default() });
            list.len() - 1
        } else {
            let key = |i: usize| self.projectors.infos.get(i).and_then(Option::as_ref).map_or((0, 0.0), |x| (x.priority, x.expiry));
            let list = &self.projectors.nodes[&node];
            let (mut best, (mut bp, mut be)) = (0, key(list[0].info));
            for (k, e) in list.iter().enumerate().skip(1) {
                let (p, ex) = key(e.info);
                if p < bp || (p == bp && ex < be) {
                    best = k;
                    bp = p;
                    be = ex;
                }
            }
            let old = list[best].info;
            self.info_release(old);
            let list = self.projectors.nodes.get_mut(&node).unwrap();
            list[best].poly = Rc::default();
            list[best].info = info;
            best
        };
        let Some(planes) = clip else { return };
        let Some(bsp) = self.level_bsp.as_ref() else { return };
        let n = &bsp.nodes[node];
        let count = n.num_vertices as usize;
        let mut poly: Vec<V> = (0..count).filter_map(|i| bsp.verts.get(n.vert_pool as usize + count - 1 - i).map(|&vi| bsp.points[vi as usize])).collect();
        for pl in planes {
            // (Z + Y) + X.
            let mut dist: Vec<f32> = poly.iter().map(|v| ((v[2] * pl[2] + v[1] * pl[1]) + v[0] * pl[0]) - pl[3]).collect();
            let mut prev = poly.len().wrapping_sub(1);
            let mut cur = 0;
            while cur < poly.len() {
                let (a, pv) = (poly[cur], poly[prev]);
                let (dc, dp) = (dist[cur], dist[prev]);
                if (0.0 < dc && dp < 0.0) || (dc < 0.0 && 0.0 < dp) {
                    // Intersection of the segment with the plane.
                    // Numerator (X + Z) + Y, denominator (Z + Y) + X.
                    let t = (pl[3] - ((pv[0] * pl[0] + pl[2] * pv[2]) + pl[1] * pv[1]))
                        / (((a[2] - pv[2]) * pl[2] + (a[1] - pv[1]) * pl[1]) + (a[0] - pv[0]) * pl[0]);
                    let i = [(a[0] - pv[0]) * t + pv[0], (a[1] - pv[1]) * t + pv[1], (a[2] - pv[2]) * t + pv[2]];
                    poly.insert(cur, i);
                    dist.insert(cur, 0.0);
                    prev = cur + 1;
                } else {
                    prev = cur;
                }
                cur = prev + 1;
            }
            let mut k = 0;
            while k < dist.len() {
                if dist[k] < 0.0 {
                    dist.remove(k);
                    poly.remove(k);
                } else {
                    k += 1;
                }
            }
        }
        let list = self.projectors.nodes.get_mut(&node).unwrap();
        if poly.is_empty() {
            let e = list.remove(slot);
            self.info_release(e.info);
        } else {
            list[slot].poly = poly.into();
        }
    }

    /// Projectors of the drawn nodes: `drawn(node)` says whether the node is rendered
    /// this frame; stale infos are removed, PF_Unlit surfaces
    /// skipped without bProjectOnUnlit.
    pub fn bsp_projector_draws(&mut self, drawn: &dyn Fn(usize) -> bool) -> Vec<ProjectorDraw> {
        let now = self.projector_now();
        let mut out = Vec::new();
        let mut nodes = std::mem::take(&mut self.projectors.draw_nodes);
        nodes.clear();
        nodes.extend(self.projectors.nodes.keys().copied());
        for &node in &nodes {
            if !drawn(node) {
                continue;
            }
            let unlit = self
                .level_bsp
                .as_ref()
                .and_then(|b| b.nodes.get(node).and_then(|n| b.surfs.get(n.surf as usize)))
                .is_some_and(|s| s.poly_flags & 0x0040_0000 != 0);
            let mut k = 0;
            while let Some(info_index) = self.projectors.nodes.get(&node).and_then(|l| l.get(k)).map(|e| e.info) {
                let valid = self.projectors.infos.get_mut(info_index).and_then(Option::as_mut).is_some_and(|info| info.still_valid(now));
                if !valid {
                    self.projectors.nodes.get_mut(&node).unwrap().remove(k);
                    self.info_release(info_index);
                    continue;
                }
                let info = self.projectors.infos[info_index].clone().unwrap();
                // a UShadowBitmapMaterial not yet drawn is not displayed.
                let shadow = self.objects.get(info.projector).is_some_and(|o| o.is_some()) && self.is_a(info.projector, "ShadowProjector");
                if shadow && !self.projectors.shadow_valid.contains(&info.projector) {
                    k += 1;
                    continue;
                }
                if !unlit || info.flags & 2 != 0 {
                    let clipped = &self.projectors.nodes[&node][k].poly;
                    let poly = if !clipped.is_empty() {
                        clipped.clone()
                    } else if let Some(poly) = self.projectors.node_polys.get(&node) {
                        poly.clone()
                    } else {
                        let poly: Rc<[V]> = self.bsp_node_polygon(node).into();
                        self.projectors.node_polys.insert(node, poly.clone());
                        poly
                    };
                    let slot = self.projectors.shadow_slots.iter().position(|s| s.is_some_and(|(p, _)| p == info.projector)).unwrap_or(0);
                    let texture = if shadow { format!("#shadow:{slot}:{}", self.projectors.shadow_intensity.get(&info.projector).copied().unwrap_or(0)) } else { crate::canvas::material_path(&info.material) };
                    out.push(ProjectorDraw { fragment: [0, node, 0], node, poly, triangles: false, texture, info });
                }
                k += 1;
            }
            if self.projectors.nodes.get(&node).is_some_and(|l| l.is_empty()) {
                self.projectors.nodes.remove(&node);
                self.projectors.node_polys.remove(&node);
            }
        }
        self.projectors.draw_nodes = nodes;
        out
    }

    /// Polygon of a BSP node, vertices in reverse order like UModel::AttachProjector's copy
    /// (that of the renderer's vertex stream).
    pub fn bsp_node_polygon(&self, node: usize) -> Vec<V> {
        let Some(bsp) = self.level_bsp.as_ref() else { return Vec::new() };
        let Some(n) = bsp.nodes.get(node) else { return Vec::new() };
        let count = n.num_vertices as usize;
        (0..count).filter_map(|i| bsp.verts.get(n.vert_pool as usize + count - 1 - i).map(|&vi| bsp.points[vi as usize])).collect()
    }
}

/// Core of UProjectorPrimitive::PointCheck: true if the box is outside none of the
/// six planes.
pub(crate) fn projector_point_core(planes: &[[f32; 4]; 6], location: V, extent: V) -> bool {
    for n in planes {
        let push = ((extent[2] * n[2]).abs() + (extent[1] * n[1]).abs()) + (extent[0] * n[0]).abs();
        let d = ((location[1] * n[1] + location[2] * n[2]) + location[0] * n[0]) - n[3];
        if d < -push {
            return false;
        }
    }
    true
}

impl ProjInfo {
    /// The info is still valid when its end of life (+0xC) is 0 or not ordered, has not
    /// passed, or the last render (+4) is at most one second old (subtraction at 24-bit precision); it then records `now`.
    pub(crate) fn still_valid(&mut self, now: f64) -> bool {
        let expired = self.expiry != 0.0 && !self.expiry.is_nan() && !(now < self.expiry) && crate::natives::x87_24(now - self.last_render) > 1.0;
        if !expired {
            self.last_render = now;
        }
        !expired
    }
}

/// Sections of a StaticMesh drawn for a projector entry without a triangle list
/// (moving projector: a growing BloodFlow…): those
/// whose material accepts projectors (`test byte [Materials + section·8 + 4], 2`, like
/// AActor::AttachProjector; without flags read, all of them) and that have primitives.
pub(crate) fn sections_projectable(mesh: &openxiii_assets::static_mesh::StaticMesh) -> Vec<usize> {
    (0..mesh.sections.len())
        .filter(|&si| !mesh.material_flags.get(si).is_some_and(|f| f & 2 == 0))
        .filter(|&si| mesh.sections[si].primitives != 0)
        .collect()
}

impl Vm {
    /// Testing: the projections attached now, by their projector's owner (name: count).
    pub fn projection_infos_debug(&self) -> String {
        let mut by: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for info in self.projectors.infos.iter().flatten() {
            let owner = self.propr(info.projector, "Owner").as_object().filter(|&o| self.alive(o));
            let name = owner.or(Some(info.projector)).and_then(|o| self.objects.get(o).and_then(|x| x.as_ref()).map(|x| x.name.to_string())).unwrap_or_default();
            *by.entry(name).or_default() += 1;
        }
        format!("PROJECTIONS t={:.1} {:?}", self.level_time, by)
    }
}
