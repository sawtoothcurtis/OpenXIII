//! Improved build only (co-op): the bulletproof vest and the helmet on a co-op player's chosen
//! character.
//!
//! The game places both items as rigid models on one bone ("X Spine1", "X Head") with offsets
//! made for XIII's body. On the other characters of the roster (character_screen.rs) they float,
//! sink in or let the body (a bigger chest, a nose, hair, a cap) come through. Nothing of the game
//! files is changed and nothing is shipped: when a co-op character first wears an item, its fit is
//! measured from the player's own game files (XIII's body, the character's body, the item's model,
//! all in the pose the meshes are made in) and kept for the run:
//!
//! - the item is moved and stretched along its own axes so that it sits on the character as it
//!   sits on XIII ([`Fit`]): the place and size of the body part it covers, measured on both
//!   bodies;
//! - what would come out through the item while it is worn (hair, a cap, a hood, a belly, a
//!   pouch) is pressed back just inside it: the drawn body is a copy of the character's model with
//!   those points moved, made the same way as the co-op arms (openxiii_assets::mod_skeletal), with
//!   the same bones and triangles (all of the hair goes under the helmet).
//!
//! Outside a co-op session nothing here runs: the solo game is the original one, XIII included
//! (XIII in co-op also keeps the game's own placement).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use openxiii_assets::coords::Coords;
use openxiii_assets::skeletal::SkeletalMesh;
use openxiii_assets::static_mesh::StaticMesh;
use openxiii_package::library::Library;

use crate::value::{ObjId, Value};
use crate::Vm;

/// The two worn items of the campaign.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// GiletMk1 (ArmorAttachment, "MeshArmesPickup.GiletHeavy" on "X Spine1").
    Vest,
    /// Casque (XIIIHelmAttachment, "MeshArmesPickup.CasqueHeavy" on "X Head").
    Helmet,
}

impl Slot {
    pub const ALL: [Slot; 2] = [Slot::Vest, Slot::Helmet];

    /// Attachment class of the item (its ThirdPersonActor).
    pub fn attachment_class(self) -> &'static str {
        match self {
            Slot::Vest => "ArmorAttachment",
            Slot::Helmet => "XIIIHelmAttachment",
        }
    }

    /// The game's values (GiletMk1 / Casque defaults): mesh, bone, RelativeLocation,
    /// RelativeRotation.
    pub fn game_placement(self) -> (&'static str, &'static str, [f32; 3], [i32; 3]) {
        match self {
            Slot::Vest => ("MeshArmesPickup.GiletHeavy", "X Spine1", [8.5, -4.0, 0.0], [-16384, 0, 0]),
            Slot::Helmet => ("MeshArmesPickup.CasqueHeavy", "X Head", [7.0, -2.0, 0.0], [-16384, 0, 0]),
        }
    }
}

/// A character's model as the analysis needs it.
pub struct Body {
    pub mesh: SkeletalMesh,
    pub bones: Vec<String>,
    pub aliases: Vec<(String, String)>,
}

/// Reads a skeletal mesh of the game ("XIIIPersos.JonesM").
pub fn read_body(library: &mut Library, path: &str) -> Result<Body, String> {
    let (loaded, index) = library.resolve(path).ok_or_else(|| format!("{path} not found"))?;
    if loaded.package.export_class(index) != "SkeletalMesh" {
        return Err(format!("{path} is not an animated mesh"));
    }
    let (p, d) = (&loaded.package, &loaded.data);
    let mesh = SkeletalMesh::read(p, d, index).map_err(|e| format!("{path}: {e}"))?;
    let bones = mesh.bones.iter().map(|b| p.name(b.name).to_string()).collect();
    let aliases = openxiii_assets::skeletal_collision::read_attach_tags(p, d, index)
        .map(|t| t.aliases.iter().zip(&t.names).map(|(&a, &n)| (p.name(a).to_string(), p.name(n).to_string())).collect())
        .unwrap_or_default();
    Ok(Body { mesh, bones, aliases })
}

/// Reads a static mesh of the game ("MeshArmesPickup.CasqueHeavy").
pub fn read_item(library: &mut Library, path: &str) -> Result<StaticMesh, String> {
    let (loaded, index) = library.resolve(path).ok_or_else(|| format!("{path} not found"))?;
    StaticMesh::read(&loaded.package, &loaded.data, index).map_err(|e| format!("{path}: {}", e.0))
}

impl Body {
    fn info(&self) -> crate::anim::MeshInfo {
        crate::anim::MeshInfo {
            default_anim: None,
            bones: self.bones.clone(),
            ref_bones: self.mesh.bones.clone(),
            scale: self.mesh.scale,
            origin: self.mesh.origin,
            rot_origin: self.mesh.rot_origin,
            aliases: self.aliases.clone(),
        }
    }

    /// Mesh → world frame of the body standing at the origin, unrotated, DrawScale 1 (the
    /// engine's actor frame with the mesh's Origin, skel.rs actor_coords).
    pub fn world_coords(&self) -> Coords {
        let info = self.info();
        let mut c = crate::skel::frame_actor([0.0; 3], [0.0; 3], [0, 0, 0], 1.0, [1.0; 3], &info);
        c.origin = [info.origin[0] + c.origin[0], info.origin[1] + c.origin[1], info.origin[2] + c.origin[2]];
        c
    }

    /// Points of the model in that world frame (the pose the model is made in).
    pub fn world_points(&self) -> Vec<[f32; 3]> {
        self.mesh.points.iter().map(|&p| self.to_world(p)).collect()
    }

    /// A point of the mesh in that world frame, as the game window places a character's points
    /// (openxiii_assets::character, actor unrotated at the origin).
    pub fn to_world(&self, p: [f32; 3]) -> [f32; 3] {
        self.mesh.transform_point_with(&openxiii_assets::level::rotation_matrix(self.mesh.rot_origin), p)
    }

    /// The mesh point drawn at world point `w` (inverse of [`Self::to_world`]).
    pub fn from_world(&self, w: [f32; 3]) -> [f32; 3] {
        let m = openxiii_assets::level::rotation_matrix(self.mesh.rot_origin);
        let s = [0, 1, 2].map(|j| w[0] * m[j][0] + w[1] * m[j][1] + w[2] * m[j][2]);
        [0, 1, 2].map(|k| s[k] / self.mesh.scale[k] + self.mesh.origin[k])
    }

    /// Bone carrying each point (its largest weight).
    pub fn main_bones(&self) -> Vec<Option<usize>> {
        self.mesh.influences.iter().map(|inf| inf.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|&(b, _)| b as usize)).collect()
    }

    /// Index of a bone by name.
    pub fn bone(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.eq_ignore_ascii_case(name))
    }

    /// True for `bone` and the bones under it.
    pub fn under(&self, bone: usize) -> Vec<bool> {
        let n = self.mesh.bones.len();
        let mut out = vec![false; n];
        for i in 0..n {
            let mut b = i;
            loop {
                if b == bone {
                    out[i] = true;
                    break;
                }
                let p = self.mesh.bones[b].parent;
                if p < 0 || p as usize == b {
                    break;
                }
                b = p as usize;
            }
        }
        out
    }

    /// The item's Location and Rotation on this body in the world frame of [`Self::world_coords`]
    /// (SetAttachmentLocation, bone_attach.rs).
    pub fn item_placement(&self, bone: &str, rel: [f32; 3], rel_rot: [i32; 3]) -> Option<([f32; 3], [i32; 3])> {
        let info = self.info();
        let tags_aliases: Vec<String> = Vec::new();
        let (b, tag) = crate::bone_attach::os_attach(&tags_aliases, &[], &[], &info.bones, bone)?;
        let bases = self.mesh.ref_space_bases();
        Some(crate::bone_attach::pose_attach(bases.get(b)?, &tag, rel, rel_rot, &self.world_coords()))
    }
}

/// The item's frame: rows = its local axes in the world (with DrawScale3D), then its origin
/// (lighting::local_to_world).
pub type Frame = [[f32; 4]; 4];

pub fn item_frame(location: [f32; 3], rotation: [i32; 3], scale3d: [f32; 3]) -> Frame {
    openxiii_assets::lighting::local_to_world(rotation, scale3d, 1.0, location, [0.0; 3])
}

/// A point of the item's model in the world.
pub fn to_world(f: &Frame, v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|k| v[0] * f[0][k] + v[1] * f[1][k] + v[2] * f[2][k] + f[3][k])
}

/// A world point in the item's own frame (unscaled frame: rotation and translation only).
pub fn to_local(f: &Frame, w: [f32; 3]) -> [f32; 3] {
    let d = [w[0] - f[3][0], w[1] - f[3][1], w[2] - f[3][2]];
    [0, 1, 2].map(|r| {
        let a = f[r];
        let l2 = a[0] * a[0] + a[1] * a[1] + a[2] * a[2];
        (d[0] * a[0] + d[1] * a[1] + d[2] * a[2]) / l2.max(1e-12)
    })
}

/// Where an item sits on a body: stretch along its own axes, then move, in its own frame (the
/// frame the game's placement gives it on that body): a point `v` of its model is drawn at
/// `v * scale + offset`. XIII's own fit is the identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub scale: [f32; 3],
    pub offset: [f32; 3],
}

impl Fit {
    pub const SAME: Fit = Fit { scale: [1.0; 3], offset: [0.0; 3] };

    pub fn apply(&self, v: [f32; 3]) -> [f32; 3] {
        [0, 1, 2].map(|k| v[k] * self.scale[k] + self.offset[k])
    }
}

/// The item's model as the analysis needs it: points and triangles.
pub struct ItemShape {
    pub points: Vec<[f32; 3]>,
    pub triangles: Vec<[usize; 3]>,
}

impl ItemShape {
    pub fn of(mesh: &StaticMesh) -> ItemShape {
        let n = mesh.positions.len();
        let triangles =
            mesh.sections.iter().flat_map(|s| mesh.section_triangles(s)).map(|t| t.map(|i| i as usize)).filter(|t| t.iter().all(|&i| i < n)).collect();
        ItemShape { points: mesh.positions.clone(), triangles }
    }
}

/// What a body does under one worn item: the item's fit, and the body's points that would come
/// out through it, pressed back inside while it is worn.
#[derive(Clone, Debug, PartialEq)]
pub struct Wear {
    pub fit: Fit,
    /// The body's points pressed inside the item (mesh space); `None`: unchanged.
    pub moved: Vec<Option<[f32; 3]>>,
}

/// Bones whose points are hair or headgear: not measured for the helmet's size.
fn hair_bone(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("ponytail") || n.contains("hair") || n.contains("chapo")
}

/// Bones of the face, the brow first, used to set the helmet's height.
const ANCHORS: &[&[&str]] = &[&["X L Eyebrow", "X R Eyebrow"], &["X L Eyebrow"], &["X R Eyebrow"], &["X Blink"], &["X Jaw"], &["X Head"]];

/// Points of a body in an item's frame (the game's placement of that item on that body).
struct Placed<'a> {
    body: &'a Body,
    /// Each point in the item's frame.
    local: Vec<[f32; 3]>,
    /// Each bone's origin in the item's frame.
    bones: Vec<[f32; 3]>,
    mains: Vec<Option<usize>>,
    /// The item's frame (unscaled) on that body.
    frame: Frame,
}

impl<'a> Placed<'a> {
    fn new(body: &'a Body, slot: Slot) -> Option<Placed<'a>> {
        let (_, bone, rel, rel_rot) = slot.game_placement();
        let (loc, rot) = body.item_placement(bone, rel, rel_rot)?;
        let f = item_frame(loc, rot, [1.0; 3]);
        let local = body.mesh.points.iter().map(|&p| to_local(&f, body.to_world(p))).collect();
        let bones = body.mesh.ref_space_bases().iter().map(|b| to_local(&f, body.to_world(b.origin))).collect();
        Some(Placed { body, local, bones, mains: body.main_bones(), frame: f })
    }

    /// Height of a set of bones (averaged), if the body has them all.
    fn height(&self, names: &[&str]) -> Option<f32> {
        let idx: Option<Vec<usize>> = names.iter().map(|n| self.body.bone(n)).collect();
        idx.map(|idx| idx.iter().map(|&i| self.bones[i][2]).sum::<f32>() / idx.len() as f32)
    }

    /// Height of the brow in the item's frame, from the first face bone the body has (`offsets`:
    /// XIII's brow above each of them).
    fn brow(&self, offsets: &[f32]) -> Option<f32> {
        ANCHORS.iter().zip(offsets).find_map(|(names, off)| self.height(names).map(|z| z + off))
    }

    /// Points whose main bone is in `set`.
    fn points_in(&self, set: &[bool]) -> Vec<usize> {
        (0..self.local.len()).filter(|&i| self.mains[i].is_some_and(|b| set.get(b).copied().unwrap_or(false))).collect()
    }
}

/// The body's parts that matter for an item: (bones measured for the fit, bones whose points
/// may be pressed inside the item, hair bones: all inside the helmet).
fn parts(body: &Body, slot: Slot) -> (Vec<bool>, Vec<bool>, Vec<bool>) {
    let n = body.bones.len();
    let head = body.bone("X Head").map_or(vec![false; n], |b| body.under(b));
    let hair: Vec<bool> = body.bones.iter().zip(&head).map(|(b, &h)| h && hair_bone(b)).collect();
    match slot {
        Slot::Helmet => {
            let measured = (0..n).map(|i| head[i] && !hair[i]).collect();
            (measured, head, hair)
        }
        Slot::Vest => {
            // The trunk: under the spine, without the head, the arms and the collarbones (the
            // shoulders come out of the vest's armholes as they should).
            let spine = body.bone("X Spine").map_or(vec![false; n], |b| body.under(b));
            let mut arms = vec![false; n];
            for name in ["X L UpperArm", "X R UpperArm"] {
                if let Some(b) = body.bone(name) {
                    for (a, u) in arms.iter_mut().zip(body.under(b)) {
                        *a |= u;
                    }
                }
            }
            let mut trunk: Vec<bool> = (0..n).map(|i| spine[i] && !head[i] && !arms[i] && !cloth_or_shoulder(&body.bones[i])).collect();
            if let Some(p) = body.bone("X Pelvis") {
                trunk[p] = true;
            }
            (trunk.clone(), trunk, vec![false; n])
        }
    }
}

/// Collarbones (the shoulders come out of the armholes) and the sleeves of a robe: not trunk.
fn cloth_or_shoulder(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("clavicle") || n.contains("manche")
}

/// Value at fraction `q` (0..1) of the values (linear between neighbours).
fn percentile(values: &mut [f32], q: f32) -> f32 {
    values.sort_by(f32::total_cmp);
    if values.is_empty() {
        return 0.0;
    }
    let x = q * (values.len() - 1) as f32;
    let (i, f) = (x.floor() as usize, x.fract());
    let j = (i + 1).min(values.len() - 1);
    values[i] + (values[j] - values[i]) * f
}

/// Low and high ends, along each axis, of the part of a body the item covers.
fn measure(placed: &Placed, slot: Slot, measured: &[bool], brow: f32) -> Option<([f32; 3], [f32; 3])> {
    let pts: Vec<[f32; 3]> = placed
        .points_in(measured)
        .into_iter()
        .map(|i| placed.local[i])
        .filter(|p| match slot {
            Slot::Vest => p[2] > -25.0 && p[2] < 22.0,
            // The band under the dome: forehead, temples, back of the skull.
            Slot::Helmet => p[2] > brow - 7.0 && p[2] < brow - 0.5,
        })
        .collect();
    if pts.len() < 8 {
        return None;
    }
    let q = match slot {
        Slot::Vest => 0.03,
        Slot::Helmet => 0.01,
    };
    let mut lo = [0.0; 3];
    let mut hi = [0.0; 3];
    for k in 0..3 {
        let mut v: Vec<f32> = pts.iter().map(|p| p[k]).collect();
        lo[k] = percentile(&mut v, q);
        hi[k] = percentile(&mut v, 1.0 - q);
    }
    Some((lo, hi))
}

/// Nearest and farthest hits of a ray (unit `d` from `o`) on the triangles, ahead of `o`.
fn ray_hits(o: [f32; 3], d: [f32; 3], pts: &[[f32; 3]], tris: &[[usize; 3]]) -> Option<(f32, f32)> {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let mut out: Option<(f32, f32)> = None;
    for t in tris {
        let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
        let (e1, e2) = (sub(b, a), sub(c, a));
        let h = cross(d, e2);
        let det = dot(e1, h);
        if det.abs() < 1e-9 {
            continue;
        }
        let f = 1.0 / det;
        let s = sub(o, a);
        let u = f * dot(s, h);
        let q = cross(s, e1);
        let v = f * dot(d, q);
        let dist = f * dot(e2, q);
        if u >= -1e-4 && v >= -1e-4 && u + v <= 1.0 + 1e-4 && dist > 1e-4 {
            out = Some(out.map_or((dist, dist), |(lo, hi)| (lo.min(dist), hi.max(dist))));
        }
    }
    out
}

/// XIII's measures, which every other body is compared with.
pub struct Reference {
    measures: [Option<([f32; 3], [f32; 3])>; 2],
    brow: f32,
    /// XIII's brow above each of [`ANCHORS`], in the helmet's frame.
    brow_offsets: Vec<f32>,
}

impl Reference {
    pub fn of(xiii: &Body) -> Option<Reference> {
        let helmet = Placed::new(xiii, Slot::Helmet)?;
        let brow = helmet.height(ANCHORS[0])?;
        let brow_offsets = ANCHORS.iter().map(|n| helmet.height(n).map_or(0.0, |z| brow - z)).collect();
        let mut measures = [None, None];
        for (i, slot) in Slot::ALL.into_iter().enumerate() {
            let placed = Placed::new(xiii, slot)?;
            measures[i] = measure(&placed, slot, &parts(xiii, slot).0, brow);
        }
        Some(Reference { measures, brow, brow_offsets })
    }
}

/// The fit of `slot`'s item on `body` and the triangles it hides, measured against XIII.
pub fn wear(reference: &Reference, body: &Body, slot: Slot, item: &ItemShape) -> Option<Wear> {
    let placed = Placed::new(body, slot)?;
    let (measured, pressed, hair) = parts(body, slot);
    let index = Slot::ALL.iter().position(|&s| s == slot)?;
    let brow = match slot {
        Slot::Helmet => placed.brow(&reference.brow_offsets)?,
        Slot::Vest => 0.0,
    };
    let (l0, h0) = reference.measures[index]?;
    let fit = match measure(&placed, slot, &measured, brow) {
        Some((l1, h1)) => {
            let mut scale = [0, 1, 2].map(|k| if (h0[k] - l0[k]).abs() > 1e-3 { (h1[k] - l1[k]) / (h0[k] - l0[k]) } else { 1.0 });
            // Never more than a third smaller or a half bigger (a model the measure misreads).
            scale = scale.map(|s| s.clamp(0.66, 1.5));
            let mut offset = [0, 1, 2].map(|k| (l1[k] + h1[k]) / 2.0 - scale[k] * (l0[k] + h0[k]) / 2.0);
            if slot == Slot::Helmet {
                // Height: the helmet keeps its proportions and sits at the same height above the
                // brow as on XIII.
                scale[2] = (scale[0] + scale[1]) / 2.0;
                offset[2] = brow - scale[2] * reference.brow;
            }
            Fit { scale, offset }
        }
        None => Fit::SAME,
    };
    let fitted: Vec<[f32; 3]> = item.points.iter().map(|&v| fit.apply(v)).collect();
    let centre = fit.offset;
    let rim = rim_profile(&fitted, centre);
    let mut moved = vec![None; placed.local.len()];
    for i in placed.points_in(&pressed) {
        let mut p = placed.local[i];
        // Hair (a ponytail, a fringe, Kim's locks) goes all under the helmet: what hangs below
        // its rim is brought up just above the rim, on that side (the hair's triangles stay
        // short), then pressed inside like the rest.
        let is_hair = placed.mains[i].is_some_and(|b| hair[b]);
        if is_hair {
            let q = [p[0] - centre[0], p[1] - centre[1]];
            let (low, reach) = rim[rim_bin(q)];
            let h = q[0].dhypot(q[1]);
            let k = if h > reach * HAIR_PULL { reach * HAIR_PULL / h } else { 1.0 };
            p = [centre[0] + q[0] * k, centre[1] + q[1] * k, p[2].max(low + 1.0)];
        }
        // From the item's axis (the vest: at the point's height) through the point.
        let o = match slot {
            Slot::Vest => [centre[0], centre[1], p[2]],
            Slot::Helmet => centre,
        };
        let mut d = [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
        if slot == Slot::Vest {
            d[2] = 0.0;
        }
        let r = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if r < 1e-6 {
            continue;
        }
        let d = d.map(|c| c / r);
        // The item is around that direction and the point comes out through it (a cap, a hood, a
        // belly): it is pressed back just inside. What is inside, or out of the item's reach (the
        // neck under the helmet's rim), stays where it is: the body keeps all its triangles.
        let Some((_, far)) = ray_hits(o, d, &fitted, &item.triangles) else {
            if is_hair {
                moved[i] = Some(body.from_world(to_world(&placed.frame, p)));
            }
            continue;
        };
        let reach = match slot {
            Slot::Vest => 1.3,
            Slot::Helmet => 3.0,
        };
        let inside = far - PRESS_INSET;
        if is_hair || (r > inside && r < far * reach && inside > 0.0) {
            let inside = if is_hair { inside.min(r).max(0.0) } else { inside };
            let l = [0, 1, 2].map(|k| o[k] + d[k] * inside);
            moved[i] = Some(body.from_world(to_world(&placed.frame, l)));
        }
    }
    Some(Wear { fit, moved })
}

/// How far from the helmet's axis hair may stay, as a share of the rim's reach on that side.
const HAIR_PULL: f32 = 0.85;

/// Directions around an item's axis for its rim profile.
const RIM_BINS: usize = 16;

fn rim_bin(q: [f32; 2]) -> usize {
    let a = q[1].datan2(q[0]);
    ((a + std::f32::consts::PI) / std::f32::consts::TAU * RIM_BINS as f32) as usize % RIM_BINS
}

/// Per direction around the item's axis: the lowest height of its edge and its farthest reach
/// (the helmet's rim seen from above).
fn rim_profile(points: &[[f32; 3]], centre: [f32; 3]) -> [(f32, f32); RIM_BINS] {
    let mut out = [(f32::MAX, 0.0f32); RIM_BINS];
    for p in points {
        let q = [p[0] - centre[0], p[1] - centre[1]];
        let r = q[0].dhypot(q[1]);
        if r > 4.0 {
            let b = &mut out[rim_bin(q)];
            *b = (b.0.min(p[2]), b.1.max(r));
        }
    }
    let lowest = out.iter().map(|b| b.0).fold(f32::MAX, f32::min).min(centre[2]);
    let widest = out.iter().map(|b| b.1).fold(0.0, f32::max);
    // A direction without a point of the item: its neighbours' values (the lowest, the widest).
    out.map(|b| if b.0 == f32::MAX { (lowest, widest) } else { b })
}

/// How far inside the item a pressed point goes (game units).
const PRESS_INSET: f32 = 0.6;

/// A body's points pressed inside an item (mesh space), by point.
type Pressed = std::rc::Rc<Vec<Option<[f32; 3]>>>;

/// What the run has measured and made (kept until the game closes: the bodies and items do not
/// change).
#[derive(Default)]
pub struct State {
    /// XIII's measures and the items' models; `None` inside: the game files lack one of them.
    base: Option<Option<std::rc::Rc<(Reference, [ItemShape; 2])>>>,
    /// Per body mesh (lower case) and item: the item's RelativeLocation and DrawScale3D on that
    /// body, and the body's points pressed inside it.
    wears: std::collections::HashMap<(String, Slot), Option<(Placement, Pressed)>>,
    /// Bodies drawn with their points pressed inside the worn items: actor → mesh drawn.
    pub(crate) drawn: std::collections::HashMap<ObjId, String>,
    /// The pressed bodies made (their names), and whether they could be.
    made: std::collections::HashMap<String, bool>,
}

/// The item's attachment values on a body: RelativeLocation, DrawScale3D.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub relative: [f32; 3],
    pub scale3d: [f32; 3],
}

/// RelativeLocation that puts the item's origin where `fit` moves it on `body` (the attachment's
/// place is linear in RelativeLocation: solved from three probes).
pub fn placement(body: &Body, slot: Slot, fit: &Fit) -> Option<Placement> {
    let (_, bone, rel, rel_rot) = slot.game_placement();
    let at = |r: [f32; 3]| body.item_placement(bone, r, rel_rot).map(|(l, _)| l);
    let (loc, rot) = body.item_placement(bone, rel, rel_rot)?;
    let target = to_world(&item_frame(loc, rot, [1.0; 3]), fit.offset);
    let base = at(rel)?;
    let mut m = [[0.0f32; 3]; 3];
    for (k, col) in m.iter_mut().enumerate() {
        let mut r = rel;
        r[k] += 1.0;
        let p = at(r)?;
        *col = [p[0] - base[0], p[1] - base[1], p[2] - base[2]];
    }
    // Columns m[k]: solve m · d = target − base (Cramer).
    let want = [target[0] - base[0], target[1] - base[1], target[2] - base[2]];
    let det3 =
        |a: [f32; 3], b: [f32; 3], c: [f32; 3]| a[0] * (b[1] * c[2] - b[2] * c[1]) - b[0] * (a[1] * c[2] - a[2] * c[1]) + c[0] * (a[1] * b[2] - a[2] * b[1]);
    let det = det3(m[0], m[1], m[2]);
    if det.abs() < 1e-6 {
        return None;
    }
    let d = [det3(want, m[1], m[2]) / det, det3(m[0], want, m[2]) / det, det3(m[0], m[1], want) / det];
    Some(Placement { relative: [rel[0] + d[0], rel[1] + d[1], rel[2] + d[2]], scale3d: fit.scale })
}

/// Name of a body drawn with its points pressed inside the items in `slots` (letters v, h).
fn pressed_name(mesh: &str, slots: &str) -> String {
    format!("Mod.CoopArmor.{}.{slots}", mesh.rsplit('.').next().unwrap_or(mesh))
}

/// The body `mesh` with the points `moved` (mesh space), as a mesh of its own with the same bones
/// (drawn with the pawn's pose and skins, as the co-op arms are). Under a helmet (`helmet`), the
/// triangles of the hair, all brought under it, are left out: those joining it to the neck would
/// otherwise be drawn stretched up into the helmet.
#[cfg(feature = "mods")]
fn make_pressed(library: &mut Library, mesh_path: &str, moved: &[Option<[f32; 3]>], helmet: bool, name: &str) -> Result<(), String> {
    use openxiii_package::ObjectRef;
    let (loaded, index) = library.resolve(mesh_path).ok_or_else(|| format!("{mesh_path} not found"))?;
    let (p, d) = (&loaded.package, &loaded.data);
    let qualified = |r: i32| match ObjectRef::from(r) {
        ObjectRef::None => String::new(),
        ObjectRef::Export(_) => format!("{}.{}", loaded.name, p.object_path(r.into())),
        _ => p.object_path(r.into()),
    };
    let mut mesh = SkeletalMesh::read(p, d, index).map_err(|e| format!("{mesh_path}: {e}"))?;
    let bone_names: Vec<String> = mesh.bones.iter().map(|b| p.name(b.name).to_string()).collect();
    let textures: Vec<String> = mesh.textures.iter().map(|&t| qualified(t)).collect();
    let animation_name = qualified(mesh.animation);
    let aliases = openxiii_assets::skeletal_collision::read_attach_tags(p, d, index)
        .map(|t| t.aliases.iter().zip(&t.names).map(|(&a, &n)| (p.name(a).to_string(), p.name(n).to_string())).collect())
        .unwrap_or_default();
    let cartoon = openxiii_assets::static_mesh::primitive_cartoon(p, d, index).map_err(|e| format!("{mesh_path}: {}", e.0))?;
    if helmet {
        let body = Body { mesh: mesh.clone(), bones: bone_names.clone(), aliases: Vec::new() };
        let head = body.bone("X Head").map_or(vec![false; bone_names.len()], |b| body.under(b));
        let hair: Vec<bool> = bone_names.iter().zip(&head).map(|(b, &h)| h && hair_bone(b)).collect();
        let mains = body.main_bones();
        let on_hair = |w: u16| mesh.wedges.get(w as usize).and_then(|w| mains.get(w.point as usize).copied().flatten()).is_some_and(|b| hair[b]);
        let keep: Vec<bool> = mesh.faces.iter().map(|f| !f.wedges.iter().any(|&w| on_hair(w))).collect();
        let mut k = keep.iter();
        mesh.faces.retain(|_| *k.next().unwrap_or(&true));
    }
    for (p, m) in mesh.points.iter_mut().zip(moved) {
        if let Some(m) = m {
            *p = *m;
        }
    }
    // Faces' part = their texture index (a mod's model).
    let materials = mesh.materials.clone();
    for f in &mut mesh.faces {
        f.material = materials.get(f.material as usize).map_or(f.material as i32, |m| m.1).max(0) as u16;
    }
    mesh.materials = (0..textures.len()).map(|t| (0, t as i32)).collect();
    for (i, b) in mesh.bones.iter_mut().enumerate() {
        b.name = i as i32;
    }
    let bounds =
        mesh.points.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), q| ([0, 1, 2].map(|k| lo[k].min(q[k])), [0, 1, 2].map(|k| hi[k].max(q[k]))));
    let bone_boxes = openxiii_assets::mod_skeletal::bone_boxes(&mesh);
    let made = openxiii_assets::mod_skeletal::ModSkeletal { mesh, bone_names, textures, animation_name, aliases, bounds, bone_boxes, cartoon, game_look: true };
    openxiii_assets::mod_skeletal::add_runtime(name, std::sync::Arc::new(made));
    Ok(())
}

/// XIII's own model: the game's placement and his whole body.
const XIII_MESH: &str = "XIIIPersos.XIIIM";

impl Vm {
    /// Once per level tick in a co-op game, after the players' characters are worn
    /// (coop_looks.rs): the vest and the helmet of each player whose body is not XIII's take
    /// that body's fit, and the body is drawn with what would come out through them pressed
    /// inside.
    pub(crate) fn coop_armor_frame(&mut self) {
        let on = !self.improved.character_screen.on && (self.coop.on || !self.improved.character_screen.remote.is_empty());
        if !on {
            self.improved.coop_armor.drawn.clear();
            return;
        }
        let pawns: Vec<ObjId> =
            self.actors.iter().copied().filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some()) && self.is_a(a, "XIIIPlayerPawn")).collect();
        let mut drawn = std::collections::HashMap::new();
        for pawn in pawns {
            let Value::Asset(mesh) = self.native_prop(pawn, crate::NativeField::Mesh).clone() else { continue };
            if mesh.eq_ignore_ascii_case(XIII_MESH) {
                continue;
            }
            let attached: Vec<ObjId> = match self.prop(pawn, "Attached") {
                Value::Array(items) => items.iter().filter_map(Value::as_object).filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())).collect(),
                _ => continue,
            };
            let mut worn = Vec::new();
            for a in attached {
                let Some(slot) = Slot::ALL
                    .into_iter()
                    .find(|s| self.objects[a].as_ref().is_some_and(|o| self.classes[o.class].name.eq_ignore_ascii_case(s.attachment_class())))
                else {
                    continue;
                };
                let Some((place, pressed)) = self.coop_armor_wear(&mesh, slot) else { continue };
                self.set_prop(a, "RelativeLocation", Value::vector(place.relative[0], place.relative[1], place.relative[2]));
                self.set_prop(a, "DrawScale3D", Value::vector(place.scale3d[0], place.scale3d[1], place.scale3d[2]));
                worn.push((slot, pressed));
            }
            if worn.is_empty() {
                continue;
            }
            worn.sort_by_key(|(s, _)| *s == Slot::Helmet);
            let letters: String = worn.iter().map(|(s, _)| if *s == Slot::Vest { 'v' } else { 'h' }).collect();
            let name = pressed_name(&mesh, &letters);
            if self.coop_armor_pressed(&mesh, &name, &worn) {
                drawn.insert(pawn, name);
            }
        }
        self.improved.coop_armor.drawn = drawn;
    }

    /// The fit of `slot` on body `mesh` (measured once per run).
    fn coop_armor_wear(&mut self, mesh: &str, slot: Slot) -> Option<(Placement, Pressed)> {
        let key = (mesh.to_ascii_lowercase(), slot);
        if let Some(w) = self.improved.coop_armor.wears.get(&key) {
            return w.clone();
        }
        let base = match &self.improved.coop_armor.base {
            Some(b) => b.clone(),
            None => {
                let library = self.library.as_mut()?;
                let made = (|| {
                    let xiii = read_body(library, XIII_MESH).ok()?;
                    let reference = Reference::of(&xiii)?;
                    let vest = ItemShape::of(&read_item(library, Slot::Vest.game_placement().0).ok()?);
                    let helmet = ItemShape::of(&read_item(library, Slot::Helmet.game_placement().0).ok()?);
                    Some(std::rc::Rc::new((reference, [vest, helmet])))
                })();
                self.improved.coop_armor.base = Some(made.clone());
                made
            }
        };
        let result = base.and_then(|b| {
            let library = self.library.as_mut()?;
            let body = read_body(library, mesh).ok()?;
            let item = &b.1[if slot == Slot::Vest { 0 } else { 1 }];
            let w = wear(&b.0, &body, slot, item)?;
            let place = placement(&body, slot, &w.fit)?;
            Some((place, std::rc::Rc::new(w.moved)))
        });
        let label = match &result {
            Some((p, m)) => format!("relative {:.2?} scale {:.3?}, {} points pressed inside", p.relative, p.scale3d, m.iter().flatten().count()),
            None => "not measured (game placement kept)".into(),
        };
        self.log.push(format!("Log: Co-op: {slot:?} on {mesh}: {label}"));
        self.improved.coop_armor.wears.insert(key, result.clone());
        result
    }

    /// Makes (once) the body `mesh` with its points pressed inside the worn items. False: drawn
    /// as it is.
    #[allow(unused_variables)]
    fn coop_armor_pressed(&mut self, mesh: &str, name: &str, worn: &[(Slot, Pressed)]) -> bool {
        if let Some(&ok) = self.improved.coop_armor.made.get(name) {
            return ok;
        }
        #[cfg(feature = "mods")]
        let ok = {
            let n = worn.iter().map(|(_, m)| m.len()).max().unwrap_or(0);
            let moved: Vec<Option<[f32; 3]>> = (0..n).map(|i| worn.iter().find_map(|(_, m)| m.get(i).copied().flatten())).collect();
            let helmet = worn.iter().any(|(s, _)| *s == Slot::Helmet);
            match self.library.as_mut().map(|l| make_pressed(l, mesh, &moved, helmet, name)) {
                Some(Ok(())) => true,
                Some(Err(e)) => {
                    self.log.push(format!("Log: Co-op: {name}: {e}"));
                    false
                }
                None => false,
            }
        };
        #[cfg(not(feature = "mods"))]
        let ok = false;
        self.improved.coop_armor.made.insert(name.to_string(), ok);
        ok
    }

    /// The mesh drawn for actor `a` instead of its own: a co-op player's body with what would come
    /// out through their vest and helmet pressed inside.
    pub fn body_mesh_override(&self, a: ObjId) -> Option<&str> {
        self.improved.coop_armor.drawn.get(&a).map(String::as_str)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        let mut v = vec![4.0, 1.0, 3.0, 2.0, 5.0];
        assert_eq!(percentile(&mut v, 0.0), 1.0);
        assert_eq!(percentile(&mut v, 0.5), 3.0);
        assert_eq!(percentile(&mut v, 1.0), 5.0);
        assert!((percentile(&mut v, 0.125) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn rays_through_a_box_face() {
        // One square (two triangles) at x = 2.
        let pts = [[2.0, -1.0, -1.0], [2.0, 1.0, -1.0], [2.0, 1.0, 1.0], [2.0, -1.0, 1.0]];
        let tris = [[0, 1, 2], [0, 2, 3]];
        assert_eq!(ray_hits([0.0; 3], [1.0, 0.0, 0.0], &pts, &tris).map(|h| h.1), Some(2.0));
        assert_eq!(ray_hits([0.0; 3], [-1.0, 0.0, 0.0], &pts, &tris), None);
        assert_eq!(ray_hits([0.0; 3], [0.0, 1.0, 0.0], &pts, &tris), None);
    }

    fn game() -> Option<Library> {
        let g = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| g.join("System").exists())?;
        Library::new(&g).ok()
    }

    /// With the game files (OXIII_GAME): XIII keeps the game's placement exactly; the other
    /// bodies get their own fit, and Jones's ponytail is pressed under the helmet.
    #[test]
    fn fits_measured_against_xiii() {
        let Some(mut library) = game() else {
            eprintln!("OXIII_GAME not set: skipped");
            return;
        };
        let xiii = read_body(&mut library, XIII_MESH).unwrap();
        let reference = Reference::of(&xiii).unwrap();
        let items: Vec<ItemShape> = Slot::ALL.iter().map(|s| ItemShape::of(&read_item(&mut library, s.game_placement().0).unwrap())).collect();
        for (slot, item) in Slot::ALL.iter().zip(&items) {
            let w = wear(&reference, &xiii, *slot, item).unwrap();
            for k in 0..3 {
                assert!((w.fit.scale[k] - 1.0).abs() < 1e-4 && w.fit.offset[k].abs() < 1e-3, "{slot:?} on XIII: {:?}", w.fit);
            }
            let p = placement(&xiii, *slot, &w.fit).unwrap();
            let rel = slot.game_placement().2;
            assert!((0..3).all(|k| (p.relative[k] - rel[k]).abs() < 1e-3), "{slot:?}: {p:?}");
        }
        let jones = read_body(&mut library, "XIIIPersos.JonesM").unwrap();
        let vest = wear(&reference, &jones, Slot::Vest, &items[0]).unwrap();
        assert!(vest.fit.scale[0] < 0.9, "a narrower vest for Jones: {:?}", vest.fit);
        let helmet = wear(&reference, &jones, Slot::Helmet, &items[1]).unwrap();
        let ponytail: Vec<usize> = jones.bones.iter().enumerate().filter(|(_, b)| b.to_ascii_lowercase().contains("ponytail")).map(|(i, _)| i).collect();
        assert!(!ponytail.is_empty());
        let mains = jones.main_bones();
        // The whole ponytail goes under the helmet; most of the body stays where it is.
        let on_ponytail: Vec<usize> = (0..jones.mesh.points.len()).filter(|&i| mains[i].is_some_and(|b| ponytail.contains(&b))).collect();
        assert!(on_ponytail.len() > 5);
        assert!(on_ponytail.iter().all(|&i| helmet.moved[i].is_some()), "a ponytail point left out of the helmet");
        assert!(helmet.moved.iter().filter(|m| m.is_some()).count() < jones.mesh.points.len() / 4);
        // Mesh space and world space are each other's inverse (the pressed points land where
        // they were placed).
        for &p in &jones.mesh.points {
            let q = jones.from_world(jones.to_world(p));
            assert!((0..3).all(|k| (p[k] - q[k]).abs() < 1e-3), "{p:?} {q:?}");
        }
        // The placement follows the fit: moved by the offset, in the item's frame.
        let p = placement(&jones, Slot::Helmet, &helmet.fit).unwrap();
        let (_, bone, rel, rot) = Slot::Helmet.game_placement();
        let (moved, r) = jones.item_placement(bone, p.relative, rot).unwrap();
        let (loc, _) = jones.item_placement(bone, rel, rot).unwrap();
        let want = to_world(&item_frame(loc, r, [1.0; 3]), helmet.fit.offset);
        assert!((0..3).all(|k| (moved[k] - want[k]).abs() < 1e-2), "{moved:?} {want:?}");
    }
}
