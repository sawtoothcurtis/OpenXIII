//! Scene visibility (UnRenderVisibility.cpp in Engine.dll): the actors that the engine
//! draws every frame, hence those that receive LastRenderTime and LastRenderDist
//! (FDynamicActor::Render). Stasis, PlayerCanSeeMe, auto-aim and
//! cinematics (CineController2) depend on it.
//!
//! - The level's display list (ULevel+0x1130, FDynamicActor of 0x80 bytes):
//!   ULevel::RefreshDisplayable at load time, AddDynDisp,
//!   RemoveDynDisp, RefreshDynDisp; FDynamicActor::Init,
//!   Refresh. Fixed actors (bStatic, !bMovable or bForceInUniverse) occupy
//!   the start of the list with 100 free slots and kept per-leaf lists; the others
//!   are placed in their leaves every frame.
//! - AActor::UpdateLeaves: BSP leaves touched by the render box.
//! - FCameraSceneNode (UpdateMatrices), FSkySceneNode,
//!   FLevelSceneNode::GetViewFrustum.
//! - Walk: stack of nodes from front to back, node boxes
//!   (iRenderBound) in the volumes of their zones, leaves, surfaces
//!   (zone portals closed by LinkedMover, anti-portals, sky zones),
//!   actors without leaves in the main volume.
//! - Volumes (FConvexVolume::BoxCheck, SphereCheck, ClipPolygon
//!   with FPoly::Split / SplitWithPlaneFast), volume of a
//!   portal, visibility interface of a zone (sphere
//!   box: at least one volume, no anti-portal that encloses it).
//! Operations in the machine code's order, in 32-bit floats (FPU on 24 bits).
//!
//! Not reimplemented: warp zones (AWarpZoneInfo) and mirrors (PF_Mirrored), absent from
//! XIII's maps; lighting of the leaf lists (FLight), drawn surfaces.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::NativeField as Field;
use crate::value::{ObjId, Value};
use crate::Vm;
use openxiii_assets::bsp::Poly;

/// Vignette of the viewport's PlayerController, read by after the translucent actors
/// (URenderTargetMaterial::Update sets it for the duration of its view).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vignette {
    /// UseVignetteFilter (+0x5B0): with VignetteColor (+0x5B4, R G B A) and
    /// VignetteMaterial (+0x5BC, path; empty: WhiteTexture).
    pub filter: Option<([u8; 4], String)>,
    /// UseVignetteHighlight (+0x5B1): bIgnoreVignetteAlpha actors drawn after the filter,
    /// then with VignetteHighLight (+0x5B8).
    pub highlight: Option<f32>,
}
use openxiii_assets::coords::{self, Matrix};

type V = [f32; 3];
type Plane = [f32; 4];

/// Surface flags read.
const PF_ANTIPORTAL: u32 = 0x10;
const PF_FAKE_BACKDROP: u32 = 0x80;
const PF_TWO_SIDED: u32 = 0x100;
const PF_PORTAL: u32 = 0x400_0000;
const PF_MIRRORED: u32 = 0x800_0000;

/// FDynamicActor: actor of the display list.
#[derive(Clone, Debug, Default)]
pub struct DynActor {
    pub actor: Option<ObjId>,
    pub refresh: Option<std::rc::Rc<DynRefresh>>,
    /// +0x44: determinant computed only by Init, kept by Refresh.
    pub init_determinant: f32,
    /// +0x48: render box (min, max), computed by Refresh.
    pub bbox: (V, V),
    /// +0x64: sphere kept by the same Refresh as the box.
    pub sphere: [f32; 4],
    /// +0x78: number of the last pass that took it.
    pub stamp: u32,
    /// +0x7C: drawn with the translucent actors (sorted by distance).
    pub translucent: bool,
    /// Last Refresh (matrix, box, sphere) and the key of its inputs: reused as long as it
    /// is the same (`refresh_dyn_bounds`).
    pub bounds_list: Option<KeptBounds>,
}

/// Key of the inputs of a Refresh's matrix, box and sphere: the actor, the
/// generation of its Props (Location, Rotation, DrawScale*, PrePivot, Mesh, StaticMesh, Brush,
/// DrawType, Collision*…) and the box of its skeleton instance (RecomputeBoundingVolume,
/// outside the Props). Meshes, StaticMeshes and brushes read by name do not change in game.
pub type KeyBounds = (ObjId, u64, Option<(V, V)>);

/// Kept result of a Refresh (without the ambience, always recomputed).
#[derive(Clone, Copy, Debug)]
pub struct KeptBounds {
    pub key_tuple: KeyBounds,
    pub matrix: Matrix,
    pub bbox: (V, V),
    pub sphere: [f32; 4],
}

/// Tests: reuses and checks done by `refresh_dyn_bounds`.
pub static BOUNDS_RESUMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Fields +4 and +74 produced together by Refresh, kept until the next
/// Refresh of this entry. Each draw keeps its own value.
#[derive(Clone, Debug, PartialEq)]
pub struct DynRefresh {
    pub matrix: Matrix,
    pub ambient: [u8; 3],
}

/// One light context per real draw pass, in its execution order.
/// None for the leaves: ActorScene reads the source indices directly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneLightSelection {
    pub light_group: u8,
    pub unlit: bool,
    /// bIgnoreDynLight (Actor+0x30 & 0x2000000): in the game (GIsEditor = 0) gives
    /// no light either; vehicle wheels.
    pub ignore_dyn: bool,
    pub leaves: Vec<i32>,
    pub sphere: [f32; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneLightDraw {
    pub actor: ObjId,
    pub refresh: Option<std::rc::Rc<DynRefresh>>,
    /// Command of the real calling draw, distinct from the Rc light selection.
    pub cull_mode: Option<u8>,
    /// Receiver of the selection before the recursive Renders.
    pub light_owner: ObjId,
    /// Selection of the base, captured before Render and shared with the attached actors.
    /// The pass context distinguishes the Source lists from the Scene lists.
    pub selection: Option<std::rc::Rc<SceneLightSelection>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneLightPass {
    /// Immutable cache of this real Scene pass; None for the legacy Source path.
    pub lights: Option<std::rc::Rc<crate::scene_light_pass::SceneLights>>,
    pub sky_zone: Option<i32>,
    pub leaves: Option<std::rc::Rc<[usize]>>,
    pub draws: Vec<SceneLightDraw>,
}

/// Display list of a level (ULevel+0x1130 to +0x1156).
#[derive(Clone, Debug, Default)]
pub struct DisplayList {
    /// Fade of each halo in each viewport of the current map.
    pub(crate) halo_fades: crate::types::HashMap<(ObjId, Option<ObjId>), (f32, f64)>,
    pub entries: Vec<DynActor>,
    /// ULevel+0x1154: start of the movable actors' part.
    pub num_static: usize,
    /// Per-leaf lists of the fixed actors (+0x1140), the last one for those without a leaf.
    pub static_leaves: Vec<Vec<usize>>,
    /// Movable actors of the frame per leaf: a leaf's list is that of the fixed ones
    /// followed by this one (the fixed ones are no longer copied every frame).
    frame_leaves: Vec<Vec<usize>>,
    /// Leaves of `frame_leaves` filled in the previous frame (the only ones emptied in the next).
    frame_touched: Vec<usize>,
    /// Per movable slot, actor and leaves read at the last build.
    mobile_membership: Vec<(Option<ObjId>, Vec<i32>)>,
    /// Only during the BSP descent, before any Render or child scene.
    mobile_refreshed: Vec<bool>,
    /// Render pass number.
    pub stamp: u32,
    /// Distinct passes, including if the same actor appears in several scenes.
    pub scene_light_passes: Vec<SceneLightPass>,
    /// HUD primitive number of the DrawPortal and RenderTarget.Update call number.
    pub portal_light_passes: std::collections::HashMap<usize, Vec<SceneLightPass>>,
    pub target_light_passes: std::collections::HashMap<u64, Vec<SceneLightPass>>,
    /// Direct DrawActor: source path, never the context of the level's scene.
    pub canvas_source_draws: Vec<SceneLightDraw>,
    /// effects_angle fix: visibility box and leaves of the movable Emitters and sprites.
    pub(crate) effects: crate::types::HashMap<ObjId, ((V, V), Vec<i32>)>,
}

/// FConvexVolume (0x204 bytes, at most 0x20 planes).
#[derive(Clone, Debug, Default)]
struct Volume {
    planes: Vec<Plane>,
}

/// Visibility interface of a zone (12 bytes): volumes through which the
/// zone is seen, anti-portals that hide it.
#[derive(Clone, Debug, Default)]
struct ZoneVis {
    volumes: Vec<Volume>,
    anti: Vec<Volume>,
}

/// FPoly reduced to what the walk uses.
#[derive(Clone, Debug, Default)]
struct FPoly {
    normal: V,
    vertices: Vec<V>,
}

/// FSceneNode / FLevelSceneNode: viewpoint of a pass.
#[derive(Clone, Debug)]
pub struct SceneNode {
    /// +0xC, +0x4C, +0x8C, +0xCC, +0x10C, +0x14C.
    view: Matrix,
    inv_view: Matrix,
    proj: Matrix,
    inv_proj: Matrix,
    world_to_screen: Matrix,
    screen_to_world: Matrix,
    /// +0x18C: ViewOrigin.
    pub origin: V,
    /// +0x1D0: zone of the viewpoint; +0x1D4: zone not to draw (-1).
    pub view_zone: i32,
    invisible_zone: i32,
    /// +0x1CC: ViewActor (bOnlyOwnerSee, bOwnerNoSee filters).
    view_actor: Option<ObjId>,
    /// +8: parent scene (sky).
    child: bool,
    /// Viewport->Actor (the PlayerController): ShowFlags, bBehindView.
    viewer: Option<ObjId>,
    show_flags: u32,
    /// Viewport+0x158 (precache): volume of zone 0 without a plane (empty FConvexVolume).
    precache: bool,
}

/// Element 's list of translucent items (from farthest to closest): an
/// actor (FDynamicActor, positive key) or a BSP node whose surface has a material to sort
/// (key −1 − node, put at the head by instead of its draw record).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Translucent {
    Node(i32),
    Actor(ObjId),
}

#[derive(Debug, Clone, Copy)]
enum TItem {
    Actor(usize, f32),
    Node(i32),
}

/// Elements of the draw lists for the actors.
#[derive(Default)]
struct Lists {
    /// Actual calls after its return from an invisible zone.
    light_leaves: Vec<usize>,
    opaque: Vec<usize>,
    translucent: Vec<TItem>,
    delayed: Vec<usize>,
    vignette: Vec<usize>,
}

/// FInverseRotationMatrix.
fn inverse_rotation_matrix(r: [i32; 3]) -> Matrix {
    let s = coords::gmath_sin;
    let sr = s(r[2].wrapping_neg());
    let cr = s(0x4000i32.wrapping_sub(r[2]));
    let sy = s(r[1].wrapping_neg());
    let cy = s(0x4000i32.wrapping_sub(r[1]));
    let sp = s(r[0].wrapping_neg());
    let cp = s(0x4000i32.wrapping_sub(r[0]));
    let mut m = [0.0f32; 16];
    m[0] = cp * cy;
    m[1] = sy * cr + sp * cy * sr;
    m[2] = sp * cy * cr - sy * sr;
    m[4] = -(cp * sy);
    m[5] = cy * cr - sp * sy * sr;
    m[6] = -(cy * sr) - sp * sy * cr;
    m[8] = -sp;
    m[9] = cp * sr;
    m[10] = cp * cr;
    m[15] = 1.0;
    m
}

pub(crate) fn translation(t: V) -> Matrix {
    let mut m = [0.0f32; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[10] = 1.0;
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m[15] = 1.0;
    m
}

/// Determinant.
fn determinant(m: &Matrix) -> f32 {
    let f2 = m[10] * m[15] - m[14] * m[11];
    let f3 = m[15] * m[6] - m[14] * m[7];
    let f4 = m[11] * m[6] - m[10] * m[7];
    let f5 = m[15] * m[2] - m[14] * m[3];
    let f6 = m[11] * m[2] - m[10] * m[3];
    let f1 = m[7] * m[2] - m[6] * m[3];
    ((f1 * m[13] + (f3 * m[1] - f5 * m[5])) * m[8] + ((f4 * m[13] + (f2 * m[5] - f3 * m[9])) * m[0] - (f6 * m[13] + (f2 * m[1] - f5 * m[9])) * m[4]))
        - (f1 * m[9] + (f4 * m[1] - f6 * m[5])) * m[12]
}

/// FMatrix::Inverse (identity if the determinant is zero).
fn inverse(m: &Matrix) -> Matrix {
    let det = determinant(m);
    if det == 0.0 {
        return translation([0.0; 3]);
    }
    let r = 1.0 / det;
    let mut o = [0.0f32; 16];
    let t7 = m[15] * m[10] - m[11] * m[14];
    let t8 = m[15] * m[6] - m[7] * m[14];
    let t1 = m[11] * m[6] - m[7] * m[10];
    o[0] = (t1 * m[13] + (t7 * m[5] - t8 * m[9])) * r;
    let t9 = m[15] * m[2] - m[14] * m[3];
    let t2 = m[11] * m[2] - m[3] * m[10];
    o[1] = -((t2 * m[13] + (t7 * m[1] - t9 * m[9])) * r);
    let t3 = m[7] * m[2] - m[6] * m[3];
    o[2] = (t3 * m[13] + (t8 * m[1] - t9 * m[5])) * r;
    o[3] = -((t3 * m[9] + (t1 * m[1] - t2 * m[5])) * r);
    o[4] = -((t1 * m[12] + (t7 * m[4] - t8 * m[8])) * r);
    o[5] = (t2 * m[12] + (t7 * m[0] - t9 * m[8])) * r;
    o[6] = -((t3 * m[12] + (t8 * m[0] - t9 * m[4])) * r);
    o[7] = (t3 * m[8] + (t1 * m[0] - t2 * m[4])) * r;
    let t7 = m[15] * m[9] - m[11] * m[13];
    let t8 = m[15] * m[5] - m[7] * m[13];
    let t1 = m[11] * m[5] - m[7] * m[9];
    o[8] = (t1 * m[12] + (t7 * m[4] - t8 * m[8])) * r;
    let t9 = m[15] * m[1] - m[13] * m[3];
    let t2 = m[11] * m[1] - m[9] * m[3];
    o[9] = -((t2 * m[12] + (t7 * m[0] - t9 * m[8])) * r);
    let t3 = m[7] * m[1] - m[5] * m[3];
    o[10] = (t3 * m[12] + (t8 * m[0] - t9 * m[4])) * r;
    o[11] = -((t3 * m[8] + (t1 * m[0] - t2 * m[4])) * r);
    let t7 = m[14] * m[9] - m[13] * m[10];
    let t8 = m[5] * m[14] - m[6] * m[13];
    let t1 = m[5] * m[10] - m[6] * m[9];
    o[12] = -((t1 * m[12] + (t7 * m[4] - t8 * m[8])) * r);
    let t9 = m[14] * m[1] - m[13] * m[2];
    let t2 = m[1] * m[10] - m[9] * m[2];
    o[13] = (t2 * m[12] + (t7 * m[0] - t9 * m[8])) * r;
    let t3 = m[6] * m[1] - m[5] * m[2];
    o[14] = -((t3 * m[12] + (t8 * m[0] - t9 * m[4])) * r);
    o[15] = (t3 * m[8] + (t1 * m[0] - t2 * m[4])) * r;
    o
}

/// FSceneNode::Project with a node's WorldToScreen matrix `m`.
pub(crate) fn project_with(m: &Matrix, v: V) -> [f32; 4] {
    let p = transform_fplane(m, [v[0], v[1], v[2], 1.0]);
    let rhw = 1.0 / p[3];
    [p[0] * rhw, p[1] * rhw, p[2] * rhw, p[3]]
}

/// FMatrix::TransformFPlane. The order of the sums is the machine code's
/// (checked on projection and deprojection).
fn transform_fplane(m: &Matrix, p: [f32; 4]) -> [f32; 4] {
    [
        ((m[12] * p[3] + m[4] * p[1]) + m[8] * p[2]) + p[0] * m[0],
        ((m[13] * p[3] + m[5] * p[1]) + m[1] * p[0]) + m[9] * p[2],
        ((m[14] * p[3] + m[6] * p[1]) + m[2] * p[0]) + m[10] * p[2],
        ((m[15] * p[3] + m[7] * p[1]) + m[3] * p[0]) + m[11] * p[2],
    ]
}

/// FVector::Normalize ((X² + Y²) + Z², false below 1e-8 compared as a double).
fn normalize(v: &mut V) -> bool {
    let s = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
    if s as f64 >= 1e-8 {
        let r = coords::rsqrt(s);
        *v = [r * v[0], r * v[1], r * v[2]];
        true
    } else {
        false
    }
}

/// FVector::SafeNormal ((X² + Y²) + Z², zero below 1e-8 compared as a double).
fn safe_normal(v: V) -> V {
    let s = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
    if (s as f64) < 1e-8 {
        return [0.0; 3];
    }
    let r = coords::rsqrt(s);
    [r * v[0], r * v[1], r * v[2]]
}

/// FPlane(A, B, C), normal ((B − A) ^ (C − A)) normalized.
/// Volume seen from `o` through a polygon: its plane, one plane per edge from
/// the origin as long as there are fewer than 0x1F planes, and the far plane at the end of the fog
/// (the far bound: DistanceFogEnd and Deproject(0, 0, 0, 5)). The W of these planes are summed
/// (Y + Z) + X, not like FPlane(A, B, C).
fn poly_volume(o: V, p: &FPoly, background: Option<(f32, V)>) -> Volume {
    let mut planes = Vec::new();
    let (n, v0) = (p.normal, p.vertices.first().copied().unwrap_or_default());
    planes.push([n[0], n[1], n[2], (v0[1] * n[1] + v0[2] * n[2]) + n[0] * v0[0]]);
    let count = p.vertices.len();
    for i in 1..=count {
        if planes.len() > 0x1e {
            break;
        }
        planes.push(plane3(o, p.vertices[i - 1], p.vertices[i % count]));
    }
    if let Some((end, c)) = background {
        let d = safe_normal([c[0] - o[0], c[1] - o[1], c[2] - o[2]]);
        let q = [d[0] * end + o[0], d[1] * end + o[1], d[2] * end + o[2]];
        planes.push([d[0], d[1], d[2], (q[1] * d[1] + q[2] * d[2]) + q[0] * d[0]]);
    }
    Volume { planes }
}

fn plane3(a: V, b: V, c: V) -> Plane {
    let mut n = [
        (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]),
        (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]),
    ];
    if !normalize(&mut n) {
        n = [0.0; 3];
    }
    // W = (N.X·A.X + N.Z·A.Z) + N.Y·A.Y (like kdop.rs; plane3 test).
    [n[0], n[1], n[2], (n[0] * a[0] + n[2] * a[2]) + n[1] * a[1]]
}

impl Volume {
    /// FConvexVolume::BoxCheck. A zone only asks for one bit of the
    /// classification: intersection for a portal, inclusion for an anti-portal.
    fn box_within_planes<const ENTIRE_BOX: bool>(&self, origin: V, extent: V) -> bool {
        self.planes.iter().all(|p| {
            // ((X + Z) + Y) − W.
            let dist = ((p[0] * origin[0] + p[2] * origin[2]) + origin[1] * p[1]) - p[3];
            let push = ((p[2] * extent[2]).abs() + (p[1] * extent[1]).abs()) + (p[0] * extent[0]).abs();
            let limit = if ENTIRE_BOX { -push } else { push };
            // Original comparison, including for an unordered distance.
            !(limit < dist)
        })
    }

    /// FConvexVolume::SphereCheck. The signed radius selects
    /// intersection or inclusion, with the same order of sums as Engine.
    fn sphere_within_planes(&self, s: [f32; 4], limit: f32) -> bool {
        self.planes.iter().all(|p| ((p[0] * s[0] + p[2] * s[2]) + s[1] * p[1]) - p[3] <= limit)
    }

    #[cfg(test)]
    fn box_check(&self, origin: V, extent: V) -> u8 {
        let intersects = self.box_within_planes::<false>(origin, extent);
        let outside = !self.box_within_planes::<true>(origin, extent);
        u8::from(intersects) | (u8::from(outside) << 1)
    }

    #[cfg(test)]
    fn sphere_check(&self, s: [f32; 4]) -> u8 {
        let intersects = self.sphere_within_planes(s, s[3].abs());
        let outside = !self.sphere_within_planes(s, -s[3]);
        u8::from(intersects) | (u8::from(outside) << 1)
    }

    /// FConvexVolume::ClipPolygon: FPoly::Split by each reversed plane.
    fn clip_polygon(&self, poly: &FPoly) -> FPoly {
        let mut p = poly.clone();
        for pl in &self.planes {
            let normal = [-pl[0], -pl[1], -pl[2]];
            let base = [pl[3] * pl[0], pl[3] * pl[1], pl[3] * pl[2]];
            if split(&mut p, normal, base) == 0 {
                p.vertices.clear();
                break;
            }
        }
        p
    }
}

/// FLinePlaneIntersection.
fn line_plane_intersection(p0: V, p1: V, pl: Plane) -> V {
    let d = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let num = pl[3] - ((p0[0] * pl[0] + pl[2] * p0[2]) + pl[1] * p0[1]);
    let den = (d[2] * pl[2] + d[1] * pl[1]) + d[0] * pl[0];
    let t = num / den;
    [d[0] * t + p0[0], d[1] * t + p0[1], d[2] * t + p0[2]]
}

/// FPoly::SplitWithPlaneFast: 0 coplanar, 1 in front, 2 behind, 3 split; the
/// front part is returned for 3.
fn split_with_plane_fast(p: &FPoly, pl: Plane, front: Option<&mut Vec<V>>) -> i32 {
    let mut sides = Vec::with_capacity(p.vertices.len());
    let (mut has_front, mut has_back) = (false, false);
    for v in &p.vertices {
        // (Y + Z) + X.
        let dist = ((pl[1] * v[1] + pl[2] * v[2]) + pl[0] * v[0]) - pl[3];
        if dist < 0.0 {
            sides.push(1);
            if dist < -0.25 {
                has_back = true;
            }
        } else {
            sides.push(0);
            if dist > 0.25 {
                has_front = true;
            }
        }
    }
    if !has_front {
        return if has_back { 2 } else { 0 };
    }
    if !has_back {
        return 1;
    }
    if let Some(out) = front {
        out.clear();
        let n = p.vertices.len();
        let mut prev_side = sides[n - 1];
        let mut prev = p.vertices[n - 1];
        for (i, &v) in p.vertices.iter().enumerate() {
            let side = sides[i];
            if side == prev_side {
                if side == 0 {
                    out.push(v);
                }
            } else {
                let x = line_plane_intersection(prev, v, pl);
                out.push(x);
                if prev_side != 0 {
                    out.push(v);
                }
            }
            prev_side = side;
            prev = v;
        }
    }
    3
}

/// FPoly::Split with NoOverflow: keeps the part in front of the plane (Base, Normal);
/// number of remaining vertices, 0 if the polygon is behind.
fn split(p: &mut FPoly, normal: V, base: V) -> usize {
    // W: (Z + Y) + X.
    let pl = [normal[0], normal[1], normal[2], (base[2] * normal[2] + base[1] * normal[1]) + normal[0] * base[0]];
    if p.vertices.len() < 14 {
        let mut front = Vec::new();
        match split_with_plane_fast(p, pl, Some(&mut front)) {
            2 => return 0,
            3 => p.vertices = front,
            _ => {}
        }
    } else if split_with_plane_fast(p, pl, None) == 2 {
        return 0;
    }
    p.vertices.len()
}

impl ZoneVis {
    /// Box in at least one volume and in no anti-portal.
    fn box_visible(&self, min: V, max: V) -> bool {
        let c = [(max[0] + min[0]) * 0.5, (max[1] + min[1]) * 0.5, (max[2] + min[2]) * 0.5];
        let e = [max[0] - c[0], max[1] - c[1], max[2] - c[2]];
        if !self.volumes.iter().any(|v| v.box_within_planes::<false>(c, e)) {
            return false;
        }
        self.anti.iter().all(|a| !a.box_within_planes::<true>(c, e))
    }

    /// Same test for a sphere.
    fn sphere_visible(&self, s: [f32; 4]) -> bool {
        if !self.volumes.iter().any(|v| v.sphere_within_planes(s, s[3].abs())) {
            return false;
        }
        self.anti.iter().all(|a| !a.sphere_within_planes(s, -s[3]))
    }
}

impl SceneNode {
    /// Viewport->Actor: the PlayerController of the view.
    pub(crate) fn viewer(&self) -> Option<ObjId> {
        self.viewer
    }

    /// FSceneNode::Deproject.
    fn deproject(&self, p: [f32; 4]) -> V {
        let r = transform_fplane(&self.screen_to_world, [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]);
        [r[0], r[1], r[2]]
    }

    /// Deproject for UInteraction's natives (ScreenToWorld).
    pub(crate) fn deproject_plane(&self, p: [f32; 4]) -> V {
        self.deproject(p)
    }

    /// FSceneNode::Project: FPlane(V, 1) × WorldToScreen (+0x10C), X, Y and Z
    /// multiplied by 1 / W, W kept.
    pub(crate) fn project(&self, v: V) -> [f32; 4] {
        project_with(&self.world_to_screen, v)
    }

    /// X and Y scales of the node's projection (+0x8C: 1 / tangent of the half angles).
    pub(crate) fn projection_scale(&self) -> [f32; 2] {
        [self.proj[0], self.proj[5]]
    }

    /// The node's WorldToScreen (+0x10C), for `project_with` later (lockstep_anchor.rs).
    pub(crate) fn world_to_screen_matrix(&self) -> Matrix {
        self.world_to_screen
    }

    /// (FMatrix::Inverse) for the callers' matrices (FCanvasUtil).
    pub(crate) fn matrix_inverse(m: &Matrix) -> Matrix {
        inverse(m)
    }
}

impl Vm {
    pub(crate) fn displayable(&self, a: ObjId) -> bool {
        !self.native_prop(a, Field::Hidden).as_bool() && self.native_prop(a, Field::DrawType).as_byte() != 0
    }

    /// Actor stored with the fixed ones: bStatic, !bMovable or bForceInUniverse.
    fn static_display(&self, a: ObjId) -> bool {
        // effects_angle fix: Emitters and sprites stored with the movable ones (box rebuilt every frame).
        if crate::effects_angle::active_value(self.engine.client.session_graphics) && self.effect_angle(a) {
            return false;
        }
        self.native_prop(a, Field::Static).as_bool() || !self.pb(a, "bMovable") || self.pb(a, "bForceInUniverse")
    }

    /// Vignette of PlayerController `controller` (ClientHighLight, ClientTargetHighLight).
    pub fn vignette(&self, controller: Option<ObjId>) -> Vignette {
        let Some(c) = controller.filter(|&c| self.alive(c) && self.is_a(c, "PlayerController")) else { return Vignette::default() };
        Vignette {
            filter: (self.propr(c, "UseVignetteFilter").as_byte() != 0)
                .then(|| (crate::canvas::color_of(&self.prop(c, "VignetteColor")), crate::canvas::material_path(&self.prop(c, "VignetteMaterial")))),
            highlight: (self.native_prop(c, Field::UseVignetteHighlight).as_byte() != 0).then(|| self.propr(c, "VignetteHighLight").as_float()),
        }
    }

    /// List of a drawn actor: deferred (bDelayDisplay; 2 with
    /// bSpecialDelayFov, otherwise 1) or 0, and bIgnoreVignetteAlpha (set aside when
    /// UseVignetteHighlight). An actor attached to bones is drawn by the FDynamicActor::Render of its
    /// base: it takes the base's list (silencer, the weapon's muzzle flash).
    pub fn draw_list(&self, a: ObjId) -> (u8, bool) {
        let a = self.render_root.get(&a).copied().filter(|&r| self.objects.get(r).is_some_and(|o| o.is_some())).unwrap_or(a);
        let delayed = match (self.native_prop(a, Field::DelayDisplay).as_bool(), self.native_prop(a, Field::SpecialDelayFov).as_bool()) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        };
        (delayed, self.native_prop(a, Field::IgnoreVignetteAlpha).as_bool())
    }

    /// Actor.Leaves.
    pub fn actor_leaves(&self, a: ObjId) -> Vec<i32> {
        self.leaf_values(a).iter().map(Value::as_int).collect()
    }

    fn leaf_values(&self, a: ObjId) -> &[Value] {
        match self.native_prop(a, Field::Leaves) {
            Value::Array(v) => v,
            _ => &[],
        }
    }

    fn num_leaves(&self) -> usize {
        self.level_bsp.as_ref().map_or(0, |m| m.leaf_zones.len())
    }

    /// AActor::UpdateLeaves: leaves touched by the render box (LocalToWorld);
    /// bVisibleOnlyOwnZone only keeps those of its zone.
    pub(crate) fn update_leaves(&mut self, a: ObjId) {
        if !self.alive(a) {
            return;
        }
        let (lo, hi) = self.render_bounding_box(a, false);
        let Some(model) = self.level_bsp.as_ref() else { return };
        // Leaves in a kept buffer (the old path makes a new list on each call).
        thread_local! {
            static BUFFER: std::cell::RefCell<Vec<i32>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        let prior = crate::physics::coll::prior(crate::physics::coll::LISTS);
        let mut leaves = if prior { model.box_leaves(lo, hi) } else { BUFFER.with(|t| std::mem::take(&mut *t.borrow_mut())) };
        if !prior {
            model.box_leaves_into(lo, hi, &mut leaves);
        }
        if self.pb(a, "bVisibleOnlyOwnZone") {
            let zone = self.region_zone_number(a);
            let zones = &self.level_bsp.as_ref().unwrap().leaf_zones;
            leaves.retain(|&l| zones.get(l as usize).copied() == Some(zone));
        }
        // Same list as before (motionless actor or box in the same leaves): nothing to rewrite.
        let same = matches!(self.native_prop(a, Field::Leaves), Value::Array(v) if v.len() == leaves.len() && v.iter().zip(&leaves).all(|(x, &l)| matches!(x, Value::Int(i) if *i == l)));
        if !same {
            self.set_native_prop(a, Field::Leaves, Value::Array(leaves.iter().map(|&l| Value::Int(l)).collect()));
        }
        if !prior {
            BUFFER.with(|t| *t.borrow_mut() = leaves);
        }
    }

    /// Is terrain `t` rendered by scene `node` (0x103DA97x-)?
    fn terrain_render(&self, node: &SceneNode, t: ObjId, zone_mask: u64) -> bool {
        let zone_kept = |zone: i32, zone_actor: Option<ObjId>| {
            zone_actor.is_some_and(|z| self.propr(z, "bTerrainZone").as_bool())
                && (node.view_zone == 0 || ((0..64).contains(&zone) && zone_mask & (1u64 << zone) != 0))
                && zone != node.invisible_zone
        };
        let zone_actor = match self.native_value(t, Field::Region) {
            Value::Struct(_, f) => f.first().and_then(|z| z.as_object()),
            _ => None,
        };
        if zone_kept(self.region_zone_number(t), zone_actor) {
            return true;
        }
        match self.propr(t, "ExtraZones") {
            Value::Array(zones) => zones.iter().filter_map(|z| z.as_object()).any(|z| zone_kept(self.region_zone_number(z), Some(z))),
            _ => false,
        }
    }

    /// Region.ZoneNumber.
    pub(crate) fn region_zone_number(&self, a: ObjId) -> i32 {
        match self.native_prop(a, Field::Region) {
            Value::Struct(_, f) => f.get(2).map_or(0, |z| z.as_int()),
            _ => 0,
        }
    }

    /// FDynamicActor::Init: Refresh, translucent, previous pass number.
    fn dyn_init(&mut self, i: usize, a: ObjId) {
        self.display.entries[i] = self.new_dyn_actor(a);
    }

    // Same construction for the real level list and the local ActorScene entry.
    // No cache per ActorId: a second Init recomputes word +44.
    fn new_dyn_actor(&mut self, a: ObjId) -> DynActor {
        let bbox = self.render_bounding_box(a, true);
        let matrix = self.dyn_init_matrix(a);
        let sphere = self.render_sphere_with_matrix(a, &matrix);
        let init_determinant = crate::dyn_init_cull::determinant3(&matrix);
        // Table: sprite (Style 3 to 5), StaticMesh (sorted material), particles and
        // trails; the other types are not translucent.
        let translucent = match self.native_prop(a, Field::DrawType).as_byte() {
            1 => (3..=5).contains(&self.native_prop(a, Field::Style).as_byte()),
            8 => self.static_mesh_sorted(a),
            10 | 12 => true,
            _ => false,
        };
        let stamp = self.display.stamp.wrapping_sub(1);
        let refresh = Some(std::rc::Rc::new(DynRefresh { matrix, ambient: self.dyn_refresh_ambient(a) }));
        DynActor { actor: Some(a), refresh, init_determinant, bbox, sphere, stamp, translucent, bounds_list: None }
    }

    // Matrix already computed by the real bounds producers: MeshToWorld
    // for DT_Mesh with a valid Mesh, otherwise LocalToWorld. No new Refresh port.
    pub(crate) fn dyn_init_matrix(&mut self, a: ObjId) -> Matrix {
        if self.native_prop(a, Field::DrawType).as_byte() == 2 {
            if let Value::Asset(path) = self.native_value(a, Field::Mesh) {
                if let Some(info) = self.mesh_info(&path) {
                    return self.mesh_to_world(a, &info);
                }
            }
        }
        crate::radius_iter::local_to_world(
            self.native_prop(a, Field::Rotation).as_rotator(),
            self.native_prop(a, Field::DrawScale3D).as_vector(),
            self.native_prop(a, Field::Location).as_vector(),
            self.native_prop(a, Field::PrePivot).as_vector(),
            self.native_prop(a, Field::DrawScale).as_float(),
        )
    }

    /// Key of the inputs of the matrix, box and sphere (see `KeyBounds`).
    fn key_bounds(&self, a: ObjId) -> Option<KeyBounds> {
        let o = self.objects.get(a)?.as_ref()?;
        let skeleton = matches!(self.native_prop(a, Field::Mesh), Value::Asset(_)).then(|| self.skel_render_boxes.get(&a).copied()).flatten();
        // Test hook: key without the Props generation.
        let generation = o.props.generation();
        Some((a, generation, skeleton))
    }

    // Refresh keeps the box then the sphere of the same actor.
    fn refresh_dyn_bounds(&mut self, i: usize, a: ObjId) {
        // Matrix, box and sphere of this entry's last Refresh reused if their inputs
        // have not changed (same actor, same Props, same skeleton box): the main view
        // from one frame to the next, a comic panel after the main view of the same frame.
        let key_tuple = self.key_bounds(a);
        let kept = key_tuple.and_then(|c| self.display.entries[i].bounds_list.filter(|b| b.key_tuple == c));
        let (matrix, bbox, sphere) = match kept {
            Some(b) => {
                BOUNDS_RESUMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                (b.matrix, b.bbox, b.sphere)
            }
            None => {
                let matrix = self.dyn_init_matrix(a);
                let bbox = self.render_box_with_matrix(a, true, Some(&matrix));
                let sphere = self.render_sphere_with_matrix(a, &matrix);
                (matrix, bbox, sphere)
            }
        };
        self.display.entries[i].bounds_list = key_tuple.map(|key_tuple| KeptBounds { key_tuple, matrix, bbox, sphere });
        let ambient = self.dyn_refresh_ambient(a);
        let refresh = DynRefresh { matrix, ambient };
        let slot = &mut self.display.entries[i].refresh;
        if let Some(current) = slot.as_mut().and_then(std::rc::Rc::get_mut) {
            *current = refresh;
        } else {
            *slot = Some(std::rc::Rc::new(refresh));
        }
        // effects_angle fix: visibility box of the particles or of the sprite's quad.
        let bbox = self.display.effects.get(&a).map_or(bbox, |(b, _)| *b);
        self.display.entries[i].bbox = bbox;
        self.display.entries[i].sphere = sphere;
    }

    fn dyn_refresh_ambient(&self, a: ObjId) -> [u8; 3] {
        use openxiii_assets::lighting::{ambient_color, zones_ambient_max_iter, ZoneAmbient};
        // Game outside the editor:.
        if self.native_prop(a, Field::IgnoreDynLight).as_bool() { return [0; 3]; }
        let color = |z: ObjId| ZoneAmbient {
            brightness: self.native_prop(z, Field::AmbientBrightness).as_byte(),
            hue: self.native_prop(z, Field::AmbientHue).as_byte(),
            saturation: self.native_prop(z, Field::AmbientSaturation).as_byte(),
        };
        let leaves = self.leaf_values(a);
        let maximum = if !self.native_prop(a, Field::UseOnlyPivotAmbient).as_bool() && leaves.len() > 1 {
            let zones = leaves.iter().filter_map(|l| self.level_bsp.as_ref()?.leaf_zones.get(l.as_int() as usize)).map(|&z| z as usize);
            zones_ambient_max_iter(zones, &|z| self.zone_actors.get(z).copied().flatten().or(self.level).map_or(ZoneAmbient::default(), color))
        } else {
            // Actor.Region.Zone, not a new BSP lookup at draw time.
            let zone = match self.native_prop(a, Field::Region) { Value::Struct(_, f) => f.first().and_then(Value::as_object), _ => None };
            zone.map_or([0.; 3], |z| zones_ambient_max_iter([0], &|_| color(z)))
        };
        ambient_color(maximum, self.native_prop(a, Field::AmbientGlow).as_byte())
    }

    fn link_static(&mut self, i: usize, a: ObjId) {
        let leaves = self.actor_leaves(a);
        let bucket = self.display.static_leaves.len().saturating_sub(1);
        if leaves.is_empty() || self.pb(a, "bForceInUniverse") {
            if let Some(l) = self.display.static_leaves.get_mut(bucket) {
                l.push(i);
            }
        } else {
            for l in leaves {
                if let Some(list) = self.display.static_leaves.get_mut(l as usize) {
                    list.push(i);
                }
            }
        }
    }

    /// ULevel::RefreshDisplayable, outside the editor: 100 free slots, the visible
    /// fixed actors, then the movable ones; leaf lists of the fixed ones.
    pub fn refresh_displayable(&mut self) {
        let actors: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.alive(a) && self.displayable(a)).collect();
        // Fixed or movable, read once: the slots are counted from it (an actor read again
        // after the Init of the ones before it could change side and overflow the list).
        let fixed: Vec<bool> = actors.iter().map(|&a| self.static_display(a)).collect();
        let statics = fixed.iter().filter(|&&f| f).count();
        let num_static = 100 + statics;
        self.display.entries = vec![DynActor::default(); num_static + actors.len() - statics];
        self.display.num_static = num_static;
        self.display.mobile_membership.clear();
        self.display.frame_leaves.clear();
        let (mut s, mut d) = (100, num_static);
        for (&a, &f) in actors.iter().zip(&fixed) {
            if f {
                self.dyn_init(s, a);
                s += 1;
            } else {
                self.dyn_init(d, a);
                d += 1;
            }
        }
        self.display.static_leaves = vec![Vec::new(); self.num_leaves() + 1];
        for i in 0..num_static {
            if let Some(a) = self.display.entries[i].actor {
                self.link_static(i, a);
            }
        }
    }

    /// ULevel::AddDynDisp.
    fn add_dyn_disp(&mut self, a: ObjId) {
        if self.static_display(a) {
            if let Some(i) = (0..self.display.num_static).find(|&i| self.display.entries[i].actor.is_none()) {
                self.dyn_init(i, a);
                self.link_static(i, a);
                return;
            }
        }
        let start = self.display.num_static.min(self.display.entries.len());
        let i = (start..self.display.entries.len()).find(|&i| self.display.entries[i].actor.is_none()).unwrap_or_else(|| {
            self.display.entries.push(DynActor::default());
            self.display.entries.len() - 1
        });
        self.dyn_init(i, a);
    }

    /// ULevel::RemoveDynDisp: the slot is freed; a fixed actor leaves the lists of the
    /// leaves it now has.
    pub(crate) fn remove_dyn_disp(&mut self, a: ObjId, at: Option<usize>) {
        let Some(i) = at.or_else(|| self.display.entries.iter().position(|e| e.actor == Some(a))) else { return };
        self.display.entries[i].actor = None;
        if i < self.display.num_static {
            let leaves = self.actor_leaves(a);
            if leaves.is_empty() {
                if let Some(l) = self.display.static_leaves.last_mut() {
                    l.retain(|&x| x != i);
                }
            } else {
                for l in leaves {
                    if let Some(list) = self.display.static_leaves.get_mut(l as usize) {
                        list.retain(|&x| x != i);
                    }
                }
            }
        }
    }

    /// ULevel::RefreshDynDisp, called by SpawnActor and AActor::RefreshDisplaying.
    pub(crate) fn refresh_dyn_disp(&mut self, a: ObjId) {
        if !self.displayable(a) {
            self.remove_dyn_disp(a, None);
            return;
        }
        if self.native_prop(a, Field::DeleteMe).as_bool() {
            return;
        }
        match self.display.entries.iter().position(|e| e.actor == Some(a)) {
            Some(i) => {
                if (i < self.display.num_static) != self.static_display(a) {
                    self.remove_dyn_disp(a, Some(i));
                    self.add_dyn_disp(a);
                } else {
                    self.refresh_dyn_bounds(i, a);
                }
            }
            None => self.add_dyn_disp(a),
        }
    }

    /// Leaf lists for the frame: the fixed actors, then each movable one in its
    /// leaves (in the last list if it has no leaf).
    fn rebuild_frame_leaves(&mut self) {
        let n = self.display.static_leaves.len().max(1);
        let first = self.display.num_static;
        let entries = &self.display.entries[first..];
        let mut membership = std::mem::take(&mut self.display.mobile_membership);
        let mut changed = self.display.frame_leaves.len() != n || membership.len() != entries.len();
        membership.resize_with(entries.len(), Default::default);
        for (entry, (previous, saved)) in entries.iter().zip(&mut membership) {
            if let Some((_, leaves)) = entry.actor.and_then(|a| self.display.effects.get(&a)) {
                // effects_angle fix: leaves of the corrected box.
                if *previous != entry.actor || saved != leaves {
                    changed = true;
                    *previous = entry.actor;
                    saved.clone_from(leaves);
                }
                continue;
            }
            let leaves = entry.actor.map_or(&[][..], |a| self.leaf_values(a));
            if *previous != entry.actor || !saved.iter().copied().eq(leaves.iter().map(Value::as_int)) {
                changed = true;
                *previous = entry.actor;
                saved.clear();
                saved.extend(leaves.iter().map(Value::as_int));
            }
        }
        self.display.mobile_membership = membership;
        if !changed {
            return;
        }
        let mut lists = std::mem::take(&mut self.display.frame_leaves);
        let mut touched = std::mem::take(&mut self.display.frame_touched);
        if lists.len() != n {
            lists.clear();
            lists.resize_with(n, Vec::new);
        } else {
            for &l in &touched {
                lists[l].clear();
            }
        }
        touched.clear();
        let bucket = n - 1;
        for (offset, (actor, leaves)) in self.display.mobile_membership.iter().enumerate() {
            if actor.is_none() { continue; }
            let i = first + offset;
            if leaves.is_empty() {
                if lists[bucket].is_empty() {
                    touched.push(bucket);
                }
                lists[bucket].push(i);
            } else {
                for &l in leaves {
                    if let Some(list) = lists.get_mut(l as usize) {
                        if list.is_empty() {
                            touched.push(l as usize);
                        }
                        list.push(i);
                    }
                }
            }
        }
        self.display.frame_leaves = lists;
        self.display.frame_touched = touched;
    }

    /// Number of actors in the frame's list for leaf `leaf` (fixed then movable), `None`
    /// outside the lists.
    fn leaf_len(&self, leaf: usize) -> Option<usize> {
        let mobiles = self.display.frame_leaves.get(leaf)?.len();
        Some(self.display.static_leaves.get(leaf).map_or(0, |s| s.len()) + mobiles)
    }

    /// k-th actor of the frame's list for leaf `leaf` (fixed then movable).
    fn leaf_entry(&self, leaf: usize, k: usize) -> usize {
        let fixed = self.display.static_leaves.get(leaf).map_or(&[][..], |s| &s[..]);
        if k < fixed.len() { fixed[k] } else { self.display.frame_leaves[leaf][k - fixed.len()] }
    }

    /// Zone actor (ULevel::GetZoneActor): Zones[z].ZoneActor, otherwise the LevelInfo.
    pub(crate) fn zone_actor(&self, z: i32) -> Option<ObjId> {
        usize::try_from(z).ok().and_then(|z| self.zone_actors.get(z).copied().flatten()).or(self.level)
    }

    /// FCameraSceneNode and UpdateMatrices, perspective and orthographic views 13/14/15.
    /// Viewport+0x158 (precache): view zone 0 instead of UModel::PointRegion.
    pub fn camera_scene(&self, viewer: Option<ObjId>, view_actor: Option<ObjId>, origin: V, rotation: [i32; 3], fov: f32, size: (u32, u32)) -> SceneNode {
        // FCameraSceneNode::UpdateMatrices: FOV × 0.5 × (π/180 in single precision).
        self.scene_node(viewer, view_actor, origin, rotation, fov, fov * 0.5 * 0.017_452_925, size)
    }

    /// `camera_scene` of a player's view on the screen (main view, split-screen view, the
    /// interactions' WorldToScreen, the HUD's DrawActor): the horizontal angle widened for a
    /// window wider than 4:3 (display_format.rs, improved build), the same scene otherwise.
    pub fn camera_scene_view(&self, viewer: Option<ObjId>, view_actor: Option<ObjId>, origin: V, rotation: [i32; 3], fov: f32, size: (u32, u32)) -> SceneNode {
        let half = fov * 0.5 * 0.017_452_925;
        let half = crate::display_format::widen_half(half, self.view_widen_window(), self.engine.client.session_graphics);
        self.scene_node(viewer, view_actor, origin, rotation, fov, half, size)
    }

    /// Window whose Hor+ factor widens the players' views (`camera_scene_view`): `display_window`,
    /// or (0, 0) (no widening) in local split screen, whose views get their angle from
    /// coop::split_view_fov (110° across a 32:9 half) and must not be widened a second time.
    pub fn view_widen_window(&self) -> (u32, u32) {
        if self.local_split() {
            return (0, 0);
        }
        self.display_window_size()
    }

    /// Window of the frame for display_format.rs: `display_window`; (0, 0) before any frame (no change).
    pub fn display_window_size(&self) -> (u32, u32) {
        let (w, h) = self.display_window;
        (w.max(0) as u32, h.max(0) as u32)
    }

    /// Scene node of a viewpoint; `half`: half opening angle in radians, computed by
    /// the caller like its own (camera: `camera_scene`; render texture: `render_target_scene`).
    pub(crate) fn scene_node(&self, viewer: Option<ObjId>, view_actor: Option<ObjId>, origin: V, rotation: [i32; 3], fov: f32, half: f32, size: (u32, u32)) -> SceneNode {
        let _ = fov;
        let view_zone = if self.precache_render { 0 } else { self.level_bsp.as_ref().map_or(0, |m| m.point_region(origin).2 as i32) };
        let mode = viewer.map_or(0, |v| self.native_prop(v, Field::RendMap).as_int());
        let ortho = matches!(mode, 13..=15);
        let mut view = translation([-origin[0], -origin[1], -origin[2]]);
        if !ortho {
            view = coords::matrix_mul(&view, &inverse_rotation_matrix(rotation));
        }
        // Screen frame (FCameraSceneNode::UpdateMatrices): X →
        // depth, Y → X × Viewport+0xAC, Z → Y × Viewport+0xB0; UViewport::UViewport
        // sets +0xAC = +0xB0 = 1.0.
        let mut swizzle = [0.0f32; 16];
        swizzle[2] = 1.0;
        swizzle[4] = 1.0;
        swizzle[9] = 1.0;
        swizzle[15] = 1.0;
        // /5B2: the editor's three planes, without the actor's rotation.
        if mode == 13 {
            swizzle = [1.0,0.0,0.0,0.0, 0.0,-1.0,0.0,0.0, 0.0,0.0,-1.0,0.0, 0.0,0.0,-origin[2],1.0];
        } else if mode == 14 {
            swizzle = [1.0,0.0,0.0,0.0, 0.0,0.0,-1.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,-origin[1],1.0];
        } else if mode == 15 {
            swizzle[14] = origin[0];
        }
        view = coords::matrix_mul(&view, &swizzle);
        let inv_view = inverse(&view);
        let (sx, sy) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let mut proj = [0.0f32; 16];
        if ortho {
            // Each multiplication/division rounded to single precision, same x87 order.
            let zoom = viewer.map_or(0.0, |v| self.propr(v, "OrthoZoom").as_float()) / (sx * 15.0);
            proj[0] = 1.0 / ((sx * zoom) * 0.5);
            proj[5] = 1.0 / ((sy * zoom) * 0.5);
            proj[10] = f32::from_bits(0x3600_0000);
            proj[14] = 0.5;
            proj[15] = 1.0;
        } else {
            // FOV/2, GetPixelRatioYX=1, planes 5 and 65536.
            let t = (half as f64).dtan();
            proj[0] = (1.0 / t) as f32;
            proj[5] = ((sx as f64 / t) as f32) / sy;
            let f = 65536.0f32 / (65536.0 - 5.0);
            proj[10] = f;
            proj[11] = 1.0;
            proj[14] = -(f * 5.0);
        }
        let inv_proj = inverse(&proj);
        let show_flags = viewer.map_or(0, |v| self.native_prop(v, Field::ShowFlags).as_int() as u32);
        SceneNode {
            world_to_screen: coords::matrix_mul(&view, &proj),
            screen_to_world: coords::matrix_mul(&inv_proj, &inv_view),
            view,
            inv_view,
            proj,
            inv_proj,
            origin,
            view_zone,
            invisible_zone: -1,
            view_actor,
            child: false,
            viewer,
            show_flags,
            precache: self.precache_render,
        }
    }

    /// FSkySceneNode: the view of the sky zone, placed at its ZoneInfo.
    fn sky_scene(&self, parent: &SceneNode, zone: i32) -> SceneNode {
        let za = self.zone_actor(zone);
        let (loc, rot) = za.map_or(([0.0; 3], [0; 3]), |z| (self.native_prop(z, Field::Location).as_vector(), self.native_prop(z, Field::Rotation).as_rotator()));
        let m = coords::matrix_mul(&coords::matrix_mul(&translation([-loc[0], -loc[1], -loc[2]]), &inverse_rotation_matrix(rot)), &translation(parent.origin));
        // FLevelSceneNode(parent, zone, matrix).
        let inv_m = inverse(&m);
        let view = coords::matrix_mul(&m, &parent.view);
        let inv_view = coords::matrix_mul(&parent.inv_view, &inv_m);
        let origin = coords::transform_fvector(&inv_m, parent.origin);
        SceneNode {
            world_to_screen: coords::matrix_mul(&view, &parent.proj),
            screen_to_world: coords::matrix_mul(&parent.inv_proj, &inv_view),
            view,
            inv_view,
            proj: parent.proj,
            inv_proj: parent.inv_proj,
            origin,
            view_zone: zone,
            invisible_zone: -1,
            view_actor: None,
            child: true,
            viewer: parent.viewer,
            show_flags: parent.show_flags,
            precache: parent.precache,
        }
    }

    /// GetViewFrustum: with SHOW_DistanceFog (0x80000), DistanceFogEnd (+0x2E0) of the
    /// view's volume if it has bDistanceFog and the PlayerController has neither bDontUseVolumeFog
    /// (+0x334 & 0x400000) nor UseVignetteFilter (+0x5B0), otherwise the zone's if bDistanceFog.
    fn fog_end(&self, node: &SceneNode) -> Option<f32> {
        if node.show_flags & 0x80000 == 0 {
            return None;
        }
        if let Some(pc) = node.viewer {
            if let Some(v) = self.view_volume(Some(pc)).filter(|&v| self.propr(v, "bDistanceFog").as_bool()) {
                if !self.propr(pc, "bDontUseVolumeFog").as_bool() && self.propr(pc, "UseVignetteFilter").as_byte() == 0 {
                    return Some(self.propr(v, "DistanceFogEnd").as_float());
                }
            }
        }
        let z = self.zone_actor(node.view_zone)?;
        self.pb(z, "bDistanceFog").then(|| self.pf(z, "DistanceFogEnd"))
    }

    /// (portal volume): DistanceFogEnd of the view's zone if it has
    /// bDistanceFog and the ShowFlags have 0x80000; the head's volume does not come into it.
    fn zone_fog_end(&self, node: &SceneNode) -> Option<f32> {
        if node.show_flags & 0x80000 == 0 {
            return None;
        }
        let z = self.zone_actor(node.view_zone)?;
        self.pb(z, "bDistanceFog").then(|| self.pf(z, "DistanceFogEnd"))
    }

    /// FLevelSceneNode::GetViewFrustum: four planes through the origin and the corners of
    /// the screen at depth 5 (widened by 0.5), the far plane (65536 or end of the fog).
    fn view_frustum(&self, node: &SceneNode) -> Volume {
        let det = determinant(&node.world_to_screen);
        let rend_map = node.viewer.map_or(0, |a| self.native_prop(a, Field::RendMap).as_int());
        if matches!(rend_map, 13..=15) {
            // Eight corners, four planes, without the far plane or fog.
            let s = [-1.0f32, 1.0];
            let mut corners = [[0.0f32; 3]; 8];
            for i in 0..2 {
                for j in 0..2 {
                    for k in 0..2 {
                        let p = transform_fplane(&node.screen_to_world, [s[i], s[j], s[k], 1.0]);
                        corners[i * 4 + j * 2 + k] = [p[0], p[1], p[2]];
                    }
                }
            }
            let triples = if !(det < 0.0) {
                [(0, 5, 1), (4, 7, 5), (6, 3, 7), (2, 1, 3)]
            } else {
                [(0, 1, 5), (4, 5, 7), (6, 7, 3), (2, 3, 1)]
            };
            return Volume { planes: triples.into_iter().map(|(a, b, c)| plane3(corners[a], corners[b], corners[c])).collect() };
        }
        let mut c = [[0.0f32; 3]; 4];
        let s = [-1.0f32, 1.0];
        for i in 0..2 {
            for j in 0..2 {
                let r = transform_fplane(&node.screen_to_world, [s[i] * 5.0, s[j] * 5.0, 0.0, 5.0]);
                c[i * 2 + j] = [r[0], r[1], r[2]];
            }
        }
        let o = node.origin;
        let mut planes = if det >= 0.0 {
            vec![plane3(o, c[0], c[1]), plane3(o, c[2], c[0]), plane3(o, c[3], c[2]), plane3(o, c[1], c[3])]
        } else {
            vec![plane3(o, c[1], c[0]), plane3(o, c[0], c[2]), plane3(o, c[2], c[3]), plane3(o, c[3], c[1])]
        };
        for p in &mut planes {
            p[3] += 0.5;
        }
        let dist = self.fog_end(node).unwrap_or(65536.0);
        let center = transform_fplane(&node.screen_to_world, [0.0, 0.0, 0.0, 5.0]);
        let d = safe_normal([center[0] - o[0], center[1] - o[1], center[2] - o[2]]);
        // W = (x1·d.X + y1·d.Y) + z1·d.Z with (x1, y1, z1) = d · dist + origin (0x103CBAxx).
        let (x1, y1, z1) = (d[0] * dist + o[0], d[1] * dist + o[1], d[2] * dist + o[2]);
        let w = (x1 * d[0] + y1 * d[1]) + z1 * d[2];
        planes.push([d[0], d[1], d[2], w]);
        Volume { planes }
    }

    /// Volume seen through a polygon (`poly_volume`); plus the far plane of the
    /// fog if `fog`.
    fn poly_volume(&self, node: &SceneNode, p: &FPoly, fog: bool) -> Volume {
        // The far plane is at the end of the fog of the view's zone alone
        // (ZoneInfo.bDistanceFog), without the head volume's one that GetViewFrustum takes.
        let background = if fog { self.zone_fog_end(node).map(|end| (end, node.deproject([0.0, 0.0, 0.0, 5.0]))) } else { None };
        poly_volume(node.origin, p, background)
    }

    /// Brush polygon of a surface, in the world (the brush's AActor::LocalToWorld).
    fn surface_poly(&self, surf: usize) -> Option<FPoly> {
        let model = self.level_bsp.as_ref()?;
        let s = model.surfs.get(surf)?;
        let actor = self.surf_actors.get(surf).copied().flatten()?;
        let poly: &Poly = self.brush_polys.get(&actor)?.get(usize::try_from(s.brush_poly).ok()?)?;
        let m = crate::radius_iter::local_to_world(
            self.native_prop(actor, Field::Rotation).as_rotator(),
            self.native_prop(actor, Field::DrawScale3D).as_vector(),
            self.native_prop(actor, Field::Location).as_vector(),
            self.native_prop(actor, Field::PrePivot).as_vector(),
            self.native_prop(actor, Field::DrawScale).as_float(),
        );
        Some(FPoly {
            normal: coords::transform_normal(&m, poly.normal),
            vertices: poly.vertices.iter().map(|&v| coords::transform_fvector(&m, v)).collect(),
        })
    }

    /// FPoly::Reverse.
    fn reverse(p: &mut FPoly) {
        p.normal = [p.normal[0] * -1.0, p.normal[1] * -1.0, p.normal[2] * -1.0];
        let n = p.vertices.len();
        for i in 0..n / 2 {
            p.vertices.swap(i, n - 1 - i);
        }
    }

    /// Open portal: without a LinkedMover, or one of its Movers open (KeyNum) or moving.
    fn portal_open(&self, actor: Option<ObjId>) -> bool {
        let Some(a) = actor else { return true };
        let open = |m: ObjId| self.native_prop(m, Field::KeyNum).as_byte() != 0 || self.pb(m, "bInterpolating");
        match self.po(a, "LinkedMover") {
            None => true,
            Some(m) if open(m) => true,
            _ => self.po(a, "LinkedMover2").is_some_and(open),
        }
    }

    /// Frame of the player's camera: FLevelSceneNode::Render.
    /// LastRenderTime and LastRenderDist of the drawn actors; `rendered` lists them in
    /// draw order.
    pub fn render_scene(&mut self, node: &SceneNode) {
        // Mods build: the camera of the view being drawn, for the HUD drawn over it
        // (mod_weapons.rs mod_tracer_size); not a scene a script draws (a canvas portal).
        #[cfg(feature = "mods")]
        if self.stack.is_empty() {
            let size = self.viewport;
            self.mods.view_projection = Some((node.clone(), size));
            if let Some(pc) = node.viewer {
                self.mods.views.insert(pc, (node.clone(), size));
            }
        }
        self.rendered.clear();
        self.display.scene_light_passes.clear();
        if self.display.entries.is_empty() && self.display.static_leaves.is_empty() {
            self.world_space_overlays(node);
            return;
        }
        // effects_angle fix: leaves and boxes of the Emitters and sprites for this frame.
        self.effects_angle_prepare();
        // Once per frame (GEngine->CurrentFrameCount): leaf lists.
        self.rebuild_frame_leaves();
        self.bsp_draw_begin();
        self.render_level(node, 0);
        self.world_space_overlays(node);
    }

    /// Lockstep co-op, drawing only: the scene of viewport `slot`'s player (`pc`, ViewActor
    /// `actor`) seen from `location` / `rotation` with the angle `fov`, the drawn camera of this
    /// machine's frame when it has turned past the step's (instant look). Its visibility
    /// replaces the scene part of the view output (put the step's back after drawing), while
    /// the game's state stays as the step left it: no LastRenderTime or LastRenderDist, no
    /// pose, sine or emitter advance, no script (`render_preview`).
    pub fn preview_scene(&mut self, slot: u8, pc: ObjId, actor: Option<ObjId>, location: V, rotation: [i32; 3], fov: f32) {
        let client = self.engine.client.lockstep_size(slot).unwrap_or(self.display_window);
        let rect = self.engine.client.viewport_rect(slot, client);
        let (w, h) = (rect[2].max(1) as u32, rect[3].max(1) as u32);
        let kept = (self.client_size, self.display_window);
        self.preview_step_drawn = self.rendered.iter().copied().collect();
        self.client_size = client;
        self.display_window = client;
        self.render_preview = true;
        let node = self.camera_scene_view(Some(pc), actor.or(Some(pc)), location, rotation, fov, (w, h));
        self.render_scene(&node);
        self.scene_bsp_views = self.bsp_views.clone();
        self.render_preview = false;
        (self.client_size, self.display_window) = kept;
    }

    /// FLevelSceneNode::Render, after: AHUD::eventWorldSpaceOverlays
    /// (no parameter) on Viewport->Actor->myHUD (+0x42C), whatever the scene (window, portal,
    /// render texture). The script only acts in debug mode (bShowDebugInfo, ShowDebugActor).
    fn world_space_overlays(&mut self, node: &SceneNode) {
        if self.render_preview {
            return;
        }
        let Some(hud) = node.viewer().and_then(|pc| self.po(pc, "myHUD")) else { return };
        self.event_pub(hud, "WorldSpaceOverlays", Vec::new());
    }

    /// For a viewpoint.
    fn render_level(&mut self, node: &SceneNode, depth: u32) {
        let state_only = self.view_state_only;
        // Made in a view never drawn too: the lights' first update of the frame draws the game's
        // random numbers (flickering lights, lighting::init_light), on every machine in the same order.
        let (mut light_snapshot, light_chain, _) = self.prepare_scene_lights(node);
        // Without a bind or a real PostLoad cache, keep the legacy consumer.
        // An available but empty cache stays Some: its empty list is meaningful.
        let light_snapshot_available = !state_only
            && self.scene_light_model.bound
            && light_snapshot.unavailable_model.is_empty()
            && !light_snapshot.unavailable_model_chain;
        let mut light_distribution = self.level_bsp.as_ref().filter(|_| !state_only).map(|model| {
            let root = if light_snapshot_available { light_chain.as_slice() } else { &[] };
            crate::scene_light_distribution::SceneLightDistribution::new(
                model, root, std::mem::take(&mut self.scene_light_model.node_lights),
            )
        }).unwrap_or_default();
        self.display.stamp = self.display.stamp.wrapping_add(1);
        let stamp = self.display.stamp;
        let mut zones: Vec<ZoneVis> = vec![ZoneVis::default(); 64];
        // Viewport+0x158 null, the frustum (GetViewFrustum) for the view
        // zone and zone 0; during precache, an empty FConvexVolume for zone 0 only.
        if node.precache {
            zones[0].volumes = vec![Volume { planes: Vec::new() }];
        } else {
            let frustum = self.view_frustum(node);
            if (0..64).contains(&node.view_zone) {
                zones[node.view_zone as usize].volumes.push(frustum.clone());
            }
            zones[0].volumes = vec![frustum];
        }
        let mut visible_zones: Vec<i32> = vec![node.view_zone];
        let mut zone_mask: u64 = if (0..64).contains(&node.view_zone) { 1u64 << node.view_zone } else { 0 };
        let mut lists = Lists::default();
        let mut children: Vec<SceneNode> = Vec::new();
        let num_surfs = self.level_bsp.as_ref().map_or(0, |m| m.surfs.len());
        let mut processed = vec![false; num_surfs];
        let rend_map = node.viewer.map_or(0, |v| self.native_prop(v, Field::RendMap).as_int());
        let has_nodes = self.level_bsp.as_ref().is_some_and(|m| !m.nodes.is_empty());
        if has_nodes && !(13..=15).contains(&rend_map) {
            // Two leaves may reject the same movable actor in different volumes.
            // No script or child view between these tests: its bounds and ambience
            // stay those of the first Refresh. Visibility is tested again each time.
            self.display.mobile_refreshed.clear();
            self.display.mobile_refreshed.resize(self.display.entries.len() - self.display.num_static, false);
            // Stack: (node, far child, far leaf, state).
            let mut stack: Vec<(i32, i32, i32, u8)> = vec![(0, -1, -1, 0)];
            while let Some(&(i_node, far_child, far_leaf, state)) = stack.last() {
                let n = self.level_bsp.as_ref().unwrap().nodes[i_node as usize].clone();
                if state == 0 {
                    if node.view_zone != 0 && n.zone_mask & zone_mask == 0 {
                        stack.pop();
                        continue;
                    }
                    if n.render_bound != -1 {
                        let (lo, hi) = self.level_bsp.as_ref().unwrap().bounds[n.render_bound as usize];
                        let seen = if node.view_zone == 0 {
                            zones[0].box_visible(lo, hi)
                        } else {
                            visible_zones.iter().any(|&z| (0..64).contains(&z) && n.zone_mask & (1u64 << z) != 0 && zones[z as usize].box_visible(lo, hi))
                        };
                        if !seen {
                            stack.pop();
                            continue;
                        }
                    }
                    // Same point as: after culling, before the children.
                    if !state_only {
                        light_distribution.node(self.level_bsp.as_ref().unwrap(),i_node as usize,&light_snapshot.lights);
                    }
                    let o = node.origin;
                    // (Z + Y) + X.
                    let u = ((o[2] * n.plane[2] + o[1] * n.plane[1]) + o[0] * n.plane[0]) - n.plane[3] > 0.0;
                    let u = u as usize;
                    let top = stack.len() - 1;
                    stack[top] = (i_node, n.child[1 - u] as i32, n.leaf[1 - u] as i32, 1);
                    if n.child[u] == -1 {
                        if n.leaf[u] != -1 {
                            self.leaf_actors(node, n.leaf[u] as i32, &zones, &mut lists, stamp);
                        }
                    } else {
                        stack.push((n.child[u] as i32, -1, -1, 0));
                    }
                } else {
                    self.node_surfaces(node, i_node, &mut zones, &mut visible_zones, &mut zone_mask, &mut processed, &mut children, depth, &mut lists);
                    let top = stack.len() - 1;
                    if far_child == -1 {
                        stack.pop();
                        if far_leaf != -1 {
                            self.leaf_actors(node, far_leaf, &zones, &mut lists, stamp);
                        }
                    } else {
                        stack[top] = (far_child, -1, -1, 0);
                    }
                }
            }
        }
        if !state_only {
            self.scene_light_model.node_lights = std::mem::take(&mut light_distribution.node_lights);
        }
        // Terrains of the level (ULevel+0x110C, loop 0x103DA97x-): rendered if the
        // actor's zone, or one of its ExtraZones, has bTerrainZone (ZoneInfo+0x1FC bit 2), is in the
        // mask of seen zones (when the view is in a zone, +0x1D0 non-zero) and is not the
        // zone not to draw (+0x1D4).
        if !self.bsp_views.is_empty() {
            let terrains: Vec<ObjId> = self.terrains.iter().map(|t| t.actor()).filter(|&t| self.terrain_render(node, t, zone_mask)).collect();
            let at = self.bsp_view_at(node.child.then_some(node.view_zone));
            self.bsp_views[at].terrains = terrains;
        }
        // Child scenes (sky), drawn after the walk, from the last created to the first:
        // Puts them at the head of its list (+0x324), walked from the head.
        for child in children.into_iter().rev() {
            self.render_level(&child, depth + 1);
        }
        // Actors without a leaf, in the main volume (root scene only). Pass number: as
        // it is after the sky scenes (each one advanced it), not the starting one.
        let stamp = self.display.stamp;
        if !node.child {
            let vignette = node.viewer.is_some_and(|v| self.native_prop(v, Field::UseVignetteHighlight).as_byte() != 0);
            let bucket = self.display.frame_leaves.len().saturating_sub(1);
            for k in 0..self.leaf_len(bucket).unwrap_or(0) {
                let i = self.leaf_entry(bucket, k);
                if self.display.entries[i].stamp == stamp {
                    continue;
                }
                if let Some(a) = self.display.entries[i].actor {
                    if !self.native_prop(a, Field::Static).as_bool() {
                        self.refresh_dyn_bounds(i, a);
                    }
                    let (lo, hi) = self.display.entries[i].bbox;
                    if zones[0].box_visible(lo, hi) {
                        if !vignette || !self.native_prop(a, Field::IgnoreVignetteAlpha).as_bool() {
                            self.push_list(node, i, a, &mut lists, false);
                        } else {
                            lists.vignette.insert(0, i);
                        }
                    }
                }
                self.display.entries[i].stamp = stamp;
            }
        }
        // Drawing: opaque, translucent, actors outside the vignette, deferred.
        let order = lists.opaque.into_iter().chain(lists.translucent.iter().filter_map(|t| match *t {
            TItem::Actor(i, _) => Some(i),
            TItem::Node(_) => None,
        }));
        // List of translucent items of the root scene, for the draw order (viewer).
        if !node.child {
            self.translucent_order = lists
                .translucent
                .iter()
                .filter_map(|t| match *t {
                    TItem::Actor(i, _) => self.display.entries[i].actor.map(Translucent::Actor),
                    TItem::Node(n) => Some(Translucent::Node(n)),
                })
                .collect();
        }
        let order = order.chain(lists.vignette).chain(lists.delayed);
        let at = (!self.bsp_views.is_empty()).then(|| self.bsp_view_at(node.child.then_some(node.view_zone)));
        let light_context: std::rc::Rc<[usize]> = lists.light_leaves.into();
        light_snapshot.leaves = light_distribution.leaf_lights;
        light_snapshot.surfaces = light_distribution.surface_lights;
        if !state_only { self.display.scene_light_passes.push(SceneLightPass { lights:light_snapshot_available.then(||std::rc::Rc::new(light_snapshot)),
            sky_zone: node.child.then_some(node.view_zone), leaves: Some(std::rc::Rc::clone(&light_context)), draws: Vec::new(),
        }); }
        for i in order {
            if let Some(a) = self.display.entries[i].actor {
                let cull = crate::dyn_init_cull::cull_mode_scene(
                    determinant(&node.world_to_screen), self.display.entries[i].init_determinant,
                );
                self.dyn_render_scene(node.origin, a, &light_context, self.display.entries[i].sphere, Some(cull), self.display.entries[i].refresh.clone());
                if let Some(at) = at {
                    self.bsp_views[at].actors.push(a);
                }
            }
        }
    }

    /// Opaque ones at the head of the list; translucent ones sorted from farthest to closest.
    /// `leaf_value`: call (actors of a leaf), which recomputes the distance of the
    /// actors already in the list as (Z² + Y²) + X²; 's loop
    /// recomputes them like its own, (X² + Y²) + Z², equal to the kept one.
    fn push_list(&mut self, node: &SceneNode, i: usize, a: ObjId, lists: &mut Lists, leaf_value: bool) {
        if !self.display.entries[i].translucent {
            lists.opaque.insert(0, i);
            return;
        }
        let l = self.native_prop(a, Field::Location).as_vector();
        let o = node.origin;
        let (dx, dy, dz) = (l[0] - o[0], l[1] - o[1], l[2] - o[2]);
        // (X² + Y²) + Z².
        let d = (dx * dx + dy * dy) + dz * dz;
        // We go past an actor that is farther (distance > d) and a surface whose plane has
        // the actor (center of its sphere, FDynamicActor+0x64) on the same side as the view (product of
        // the signed distances > 0); otherwise we insert in front.
        // Center of the actor's sphere (FDynamicActor+0x64, FSphere::TransformBy of
        // GetRenderBoundingSphere in Refresh), not that of its box.
        let need = lists.translucent.iter().any(|t| matches!(t, TItem::Node(_)));
        let c = if need {
            let s = self.render_bounding_sphere(a);
            [s[0], s[1], s[2]]
        } else {
            [0.0; 3]
        };
        let at = lists
            .translucent
            .iter()
            .position(|t| match *t {
                TItem::Actor(j, dj) => {
                    if !leaf_value {
                        return dj <= d;
                    }
                    let Some(b) = self.display.entries[j].actor else { return dj <= d };
                    let l = self.native_prop(b, Field::Location).as_vector();
                    let (dx, dy, dz) = (l[0] - o[0], l[1] - o[1], l[2] - o[2]);
                    (dz * dz + dy * dy) + dx * dx <= d
                }
                TItem::Node(n) => {
                    let Some(pl) = self.level_bsp.as_ref().and_then(|m| m.surfs.get(m.nodes.get(n as usize)?.surf as usize)).map(|s| s.plane) else { return true };
                    // (Z + Y) + X for both.
                    let so = ((o[2] * pl[2] + o[1] * pl[1]) + o[0] * pl[0]) - pl[3];
                    let its = ((c[2] * pl[2] + c[1] * pl[1]) + c[0] * pl[0]) - pl[3];
                    so * its <= 0.0
                }
            })
            .unwrap_or(lists.translucent.len());
        lists.translucent.insert(at, TItem::Actor(i, d));
    }

    /// Actors of a leaf not yet taken in this pass, within the volume of
    /// the leaf's zone, filtered by owner.
    fn leaf_actors(&mut self, node: &SceneNode, leaf: i32, zones: &[ZoneVis], lists: &mut Lists, stamp: u32) {
        let zone = if node.view_zone == 0 { 0 } else { self.level_bsp.as_ref().and_then(|m| m.leaf_zones.get(leaf as usize).copied()).unwrap_or(0) };
        if zone == node.invisible_zone {
            return;
        }
        // Lights are added even without an actor in the leaf.
        lists.light_leaves.push(leaf as usize);
        let Some(len) = self.leaf_len(leaf as usize) else { return };
        if len == 0 { return; }
        let vis = &zones[zone.clamp(0, 63) as usize];
        let vignette = node.viewer.is_some_and(|v| self.native_prop(v, Field::UseVignetteHighlight).as_byte() != 0);
        let behind = node.viewer.is_some_and(|v| self.native_prop(v, Field::BehindView).as_bool());
        for k in 0..len {
            let i = self.leaf_entry(leaf as usize, k);
            if self.display.entries[i].stamp == stamp {
                continue;
            }
            if i >= self.display.num_static {
                if let Some(a) = self.display.entries[i].actor {
                    let already_refreshed = self.display.mobile_refreshed.get_mut(i - self.display.num_static)
                        .is_some_and(|seen| std::mem::replace(seen, true));
                    if !already_refreshed { self.refresh_dyn_bounds(i, a); }
                }
            }
            let (lo, hi) = self.display.entries[i].bbox;
            let visible = vis.box_visible(lo, hi);
            if !visible {
                continue;
            }
            if let Some(a) = self.display.entries[i].actor {
                let owned = |vm: &Vm| node.view_actor.is_some_and(|v| vm.is_owned_by(a, v));
                let only_owner = self.native_prop(a, Field::OnlyOwnerSee).as_bool();
                let owner_no_see = self.native_prop(a, Field::OwnerNoSee).as_bool();
                if (!only_owner || owned(self)) && (!owner_no_see || !owned(self) || behind) {
                    if self.native_prop(a, Field::DelayDisplay).as_bool() {
                        lists.delayed.insert(0, i);
                    } else if !vignette || !self.native_prop(a, Field::IgnoreVignetteAlpha).as_bool() {
                        self.push_list(node, i, a, lists, true);
                    } else {
                        lists.vignette.insert(0, i);
                    }
                }
            }
            self.display.entries[i].stamp = stamp;
        }
    }

    /// Surfaces of a node and of its coplanar nodes (zone portals, anti-portals,
    /// sky zones).
    #[allow(clippy::too_many_arguments)]
    fn node_surfaces(
        &mut self,
        node: &SceneNode,
        mut i_node: i32,
        zones: &mut [ZoneVis],
        visible_zones: &mut Vec<i32>,
        zone_mask: &mut u64,
        processed: &mut [bool],
        children: &mut Vec<SceneNode>,
        depth: u32,
        lists: &mut Lists,
    ) {
        while i_node != -1 {
            let Some(n) = self.level_bsp.as_ref().and_then(|m| m.nodes.get(i_node as usize)).cloned() else { return };
            let next = n.coplanar as i32;
            let surf_index = n.surf as usize;
            let Some(surf) = self.level_bsp.as_ref().and_then(|m| m.surfs.get(surf_index)).cloned() else {
                i_node = next;
                continue;
            };
            // Zone numbers are bytes from the map file: one past the zone table comes from a damaged map,
            // the node is left out instead of indexing past the table.
            if n.zone.iter().any(|&z| z as usize >= zones.len()) {
                i_node = next;
                continue;
            }
            let o = node.origin;
            // (Y + Z) + X.
            let u = (0.0 < ((o[1] * n.plane[1] + o[2] * n.plane[2]) + o[0] * n.plane[0]) - n.plane[3]) as usize;
            let flags = surf.poly_flags;
            if u == 0 && flags & (PF_PORTAL | PF_TWO_SIDED) == 0 && node.view_zone != 0 {
                i_node = next;
                continue;
            }
            let zone = if node.view_zone == 0 { 0 } else { n.zone[u] as usize };
            if !zones[zone].sphere_visible(n.sphere) {
                i_node = next;
                continue;
            }
            let actor = self.surf_actors.get(surf_index).copied().flatten();
            if flags & PF_FAKE_BACKDROP != 0 {
                // Sky zone of the node's zone (ZoneInfo.SkyZone), one scene per zone; it requires
                // SHOW_Backdrop (0x800) and 4 in the ShowFlags (Viewport+0x158 is null in game).
                let sky_flags = node.show_flags & 4 != 0 && node.show_flags & 0x800 != 0;
                // No depth limit in the original (a sky can open a sky, up to 6 levels observed in the
                // game's maps); 64 only avoids a stack overflow where the original crashes.
                if sky_flags && depth < 64 {
                    if let Some(sky) = self.zone_actor(n.zone[u] as i32).and_then(|z| self.po(z, "SkyZone")) {
                        let sz = self.region_zone_number(sky);
                        if !children.iter().any(|c| c.view_zone == sz) {
                            children.push(self.sky_scene(node, sz));
                        }
                    }
                }
            } else if flags & PF_MIRRORED != 0 {
                // Mirrors (FMirrorSceneNode): not reimplemented.
            } else if node.view_zone != 0 && flags & PF_PORTAL != 0 {
                // A warp zone on the other side (AWarpZoneInfo) is not reimplemented.
                let warp = self.zone_actor(n.zone[1 - u] as i32).is_some_and(|z| self.is_a(z, "WarpZoneInfo"));
                if self.portal_open(actor) && !warp {
                    let near = n.zone[u] as usize;
                    let far = n.zone[1 - u] as usize;
                    if actor.is_some() && !processed[surf_index] && n.zone[0] != n.zone[1] {
                        if let Some(mut poly) = self.surface_poly(surf_index) {
                            if u == 0 {
                                Self::reverse(&mut poly);
                            }
                            let sources = zones[near].volumes.clone();
                            for flight in sources {
                                let clipped = flight.clip_polygon(&poly);
                                if !clipped.vertices.is_empty() {
                                    let v = self.poly_volume(node, &clipped, true);
                                    zones[far].volumes.push(v);
                                }
                            }
                            if *zone_mask & (1u64 << far) == 0 {
                                visible_zones.insert(0, far as i32);
                                *zone_mask |= 1u64 << far;
                            }
                        }
                        processed[surf_index] = true;
                    }
                }
            } else if flags & PF_ANTIPORTAL != 0 && actor.is_some() && !processed[surf_index] {
                if let Some(mut poly) = self.surface_poly(surf_index) {
                    if let Some(&v0) = poly.vertices.first() {
                        let nn = poly.normal;
                        // (Z + Y) + X.
                        if (nn[2] * (o[2] - v0[2]) + nn[1] * (o[1] - v0[1])) + (o[0] - v0[0]) * nn[0] < 0.0 {
                            Self::reverse(&mut poly);
                        }
                        let v = self.poly_volume(node, &poly, false);
                        zones[n.zone[u] as usize].anti.push(v);
                    }
                    processed[surf_index] = true;
                }
            }
            // Material to sort (UMaterial::RequiresSorting, call +0x78): at the head of the list of
            // translucent items rather than in the draw record.
            // Portal without an Actor: neither volume nor passage, but [esp+0x10] stays 1 and the node is
            // drawn even when marked PF_Invisible.
            let portal_without_actor = node.view_zone != 0 && flags & PF_PORTAL != 0 && actor.is_none();
            if self.bsp_draw_node(node.child, i_node, u, node.view_zone, node.invisible_zone, node.show_flags, node.viewer, portal_without_actor) == Some(true) {
                lists.translucent.insert(0, TItem::Node(i_node));
            }
            i_node = next;
        }
    }

    /// End of the scene walk: the Render and its attached actors use the same light heads
    /// built, kept for the draw consumer.
    fn dyn_render_scene(&mut self, origin: V, a: ObjId, treated: &std::rc::Rc<[usize]>, sphere: [f32; 4], cull_mode: Option<u8>, refresh: Option<std::rc::Rc<DynRefresh>>) {
        if self.view_state_only {
            // A view never drawn: the actor's render for the game (dyn_render), no light pass.
            let mut snapshots = Vec::new();
            self.dyn_render(origin, a, 0, refresh, &mut snapshots);
            self.render_root.extend(snapshots.iter().map(|&(b, _)| (b, a)));
            return;
        }
        let selection = self.scene_light_selection(a, sphere);
        let mut snapshots = Vec::new();
        self.dyn_render(origin, a, 0, refresh, &mut snapshots);
        self.render_root.extend(snapshots.iter().map(|&(b, _)| (b, a)));
        if let Some(pass) = self.display.scene_light_passes.last_mut() {
            if pass.leaves.as_ref().is_some_and(|leaves| std::rc::Rc::ptr_eq(leaves, treated)) {
                pass.draws.extend(snapshots.into_iter().enumerate().map(|(draw_index, (actor, refresh))| SceneLightDraw { actor, refresh, cull_mode: cull_mode.filter(|_| draw_index == 0), light_owner: a, selection: Some(std::rc::Rc::clone(&selection)) }));
            }
        }
    }

    /// Light selection of the drawn actor. The previous frame's one is reused if
    /// its values are the same to the bit (drawing only reads its content): no new Vec or
    /// Rc for an actor that has not moved.
    fn scene_light_selection(&mut self, a: ObjId, sphere: [f32; 4]) -> std::rc::Rc<SceneLightSelection> {
        let light_group = self.native_prop(a, Field::StaticLightGroup).as_int() as u8;
        let unlit = self.native_prop(a, Field::Unlit).as_bool();
        let ignore_dyn = self.native_prop(a, Field::IgnoreDynLight).as_bool();
        if let Some(s) = self.light_selections.get(&a) {
            let leaves = self.leaf_values(a);
            if s.light_group == light_group && s.unlit == unlit && s.ignore_dyn == ignore_dyn && s.sphere.map(f32::to_bits) == sphere.map(f32::to_bits)
                && s.leaves.len() == leaves.len() && s.leaves.iter().zip(leaves).all(|(&l, v)| l == v.as_int()) {
                return std::rc::Rc::clone(s);
            }
        }
        let s = std::rc::Rc::new(SceneLightSelection { light_group, unlit, ignore_dyn, leaves: self.actor_leaves(a), sphere });
        self.light_selections.insert(a, std::rc::Rc::clone(&s));
        s
    }

    /// FDynamicActor::Render for what affects the logic: LastRenderTime =
    /// Level.TimeSeconds, LastRenderDist, then the actors attached to the bones of a skeletal mesh
    /// (placed, drawn, bHidden).
    fn dyn_render(&mut self, origin: V, a: ObjId, depth: u32, refresh: Option<std::rc::Rc<DynRefresh>>, snapshots: &mut Vec<(ObjId, Option<std::rc::Rc<DynRefresh>>)>) {
        if !self.objects.get(a).is_some_and(|o| o.is_some()) {
            return;
        }
        let t = self.level.map_or(self.level_time as f32, |l| self.native_prop(l, Field::TimeSeconds).as_float());
        let preview = self.render_preview;
        // Without changing the Props generation (Props::set_render).
        if let Some(i) = self.native_slot_of(a, Field::LastRenderTime).filter(|_| !preview) {
            if let Some(o) = self.objects[a].as_mut() { o.props.set_render(i, Value::Float(t)); }
        }
        let l = self.native_prop(a, Field::Location).as_vector();
        let o = origin;
        let (dz, dy, dx) = (o[2] - l[2], o[1] - l[1], o[0] - l[0]);
        // dist² = (dx² + dy²) + dz² (x first), appSqrt as a double.
        let dist = (((dx * dx + dy * dy) + dz * dz) as f64).sqrt() as f32;
        if let Some(i) = self.native_slot_of(a, Field::LastRenderDist).filter(|_| !preview) {
            if let Some(o) = self.objects[a].as_mut() { o.props.set_render(i, Value::Float(dist)); }
        }
        self.rendered.push(a);
        let mut p = crate::render_placement::RenderPlacement::read(self, a);
        if !preview {
            self.first_render.entry(a).or_insert(p);
        }
        // Network play: a remote player's pawn and what it carries (weapon, vest, helmet) drawn
        // smoothly between its moves (net_smooth.rs); the actor's place is not changed. Static
        // meshes are drawn with the refresh matrix: it moves by the same offset, or a worn item
        // would be drawn at the true place while the body is drawn at the smoothed one.
        let mut refresh = refresh;
        if let Some(o) = self.net_render_offset(a) {
            p.location = std::array::from_fn(|k| p.location[k] + o[k]);
            let shift = true;
            if let Some(r) = refresh.as_mut().filter(|_| shift) {
                let m = std::rc::Rc::make_mut(r);
                for k in 0..3 {
                    m.matrix[12 + k] += o[k];
                }
            }
        }
        snapshots.push((a, refresh));
        self.render_place.insert(a, p);
        // DT_Mesh with an instance: USkeletalMeshInstance::Render, which calls
        // GetFrame on every frame where the actor is drawn: cached pose, tween (tween_base) and root
        // then advance even if nothing else asks for the pose in the frame.
        if !preview && self.native_prop(a, Field::DrawType).as_byte() == 2 && self.mesh_instance(a).is_some() {
            self.get_frame_without_render(a);
        }
        // (DT_StaticMesh with a StaticMesh): advance of its USinusModifiers.
        if !preview {
            self.sine_render(a);
        }
        // AEmitter::Render: each drawn UParticleEmitter becomes active again (Inactive = 0).
        if !preview && self.is(a, crate::flags_classes::D::Emitter) {
            self.emitter_rendered(a);
        }
        if depth < 4 {
            let place = !preview;
            let attached = if place { self.render_bone_attachments(a) } else { self.preview_bone_attachments(a) };
            for b in attached {
                let local = self.new_dyn_actor(b);
                self.dyn_render(origin, b, depth + 1, local.refresh, snapshots);
            }
        }
    }

    /// UCanvas::execDrawActor: the player's PlayerCalcView, then FActorSceneNode::Render,
    /// which draws the actor alone (FDynamicActor::Init then Render). `wire`: the viewport's RendMap set to 1
    /// for the duration of the draw, restored afterwards; `clear_z`: the depth is cleared before (Clear of
    /// the render interface); `fov`: DisplayFOV, failing that the player's FOVAngle (+0x1F8 of Viewport->Actor).
    pub(crate) fn canvas_draw_actor(&mut self, a: ObjId, wire: bool, clear_z: bool, fov: Option<f32>) {
        let Some(pc) = self.render_player else { return };
        let fov = fov.unwrap_or_else(|| self.pf(pc, "FOVAngle"));
        self.canvas_actor_calls.push((a, wire, clear_z, fov));
        let rend_map = self.native_prop(pc, Field::RendMap).clone();
        if wire {
            self.set_prop(pc, "RendMap", Value::Int(1));
        }
        let mut origin = self.native_prop(pc, Field::Location).as_vector();
        let mut rotation = self.native_prop(pc, Field::Rotation).as_rotator();
        if let Some(f) = self.find_function(pc, "PlayerCalcView", false) {
            let args = vec![Some(Value::Object(Some(pc))), Some(self.native_value(pc, Field::Location)), Some(self.native_value(pc, Field::Rotation))];
            if let Ok((_, outs)) = self.call(f, pc, args) {
                if let Some(v) = outs.get(1).cloned().flatten() {
                    origin = v.as_vector();
                }
                if let Some(v) = outs.get(2).cloned().flatten() { rotation = v.as_rotator(); }
            }
        }
        if self.alive(a) {
            let local = self.new_dyn_actor(a);
            let size = (self.client_size.0.max(1) as u32, self.client_size.1.max(1) as u32);
            let node = self.camera_scene_view(Some(pc), Some(a), origin, rotation, fov, size);
            let cull_mode = Some(crate::dyn_init_cull::cull_mode_scene(
                determinant(&node.world_to_screen), local.init_determinant,
            ));
            // FActorSceneNode::Render: Init of the base then
            // a single selection before the recursive Render. bUnlit is not read here.
            let rend = self.native_prop(pc, Field::RendMap).as_int();
            let lighting = rend == 5 || rend == 7 || (rend == 16 && self.native_prop(pc, Field::ShowFlags).as_int() != 0);
            let selection = std::rc::Rc::new(SceneLightSelection {
                light_group: self.native_prop(a, Field::StaticLightGroup).as_int() as u8,
                unlit: false,
                // FActorSceneNode::Render does not call.
                ignore_dyn: false,
                leaves: if lighting { self.actor_leaves(a) } else { Vec::new() },
                sphere: self.render_bounding_sphere(a),
            });
            let start = self.rendered.len();
            let mut snapshots = Vec::new();
            self.dyn_render(origin, a, 0, local.refresh, &mut snapshots);
            self.render_root.extend(snapshots.iter().map(|&(b, _)| (b, a)));
            let drawn: Vec<ObjId> = self.rendered.drain(start..).collect();
            self.display.canvas_source_draws.extend(snapshots.into_iter().enumerate().map(|(draw_index, (actor, refresh))| SceneLightDraw { actor, refresh, cull_mode: cull_mode.filter(|_| draw_index == 0), light_owner: a, selection: Some(std::rc::Rc::clone(&selection)) }));
            self.canvas_rendered.extend(drawn);
        }
        if wire {
            self.set_prop(pc, "RendMap", rend_map);
        }
    }
}

