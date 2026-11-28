//! Engine.dll's BSP model (UModel) for collision and zones: volume brushes and
//! the map's BSP. The UnTrace.cpp functions are copied one by one from the machine code:
//! UModel::PointCheck, UModel::LineCheck, UModel::PointRegion,
//! with the brush frames (ABrush::ToLocal, ABrush::ToWorld,
//! FCoords * FScale, FCoords / FScale).
//!
//! Reading (UModel::Serialize, version 100, licensee 58) up to RootOutside:
//! UPrimitive (FBox, FSphere, 4 + 4 bytes); Vectors; Points; Nodes (70 bytes: plane, ZoneMask,
//! NodeFlags, iVertPool, iSurf, iBack, iFront, iPlane, iCollisionBound i32, iRenderBound, sphere,
//! iZone[2], NumVertices, iLeaf[2], 2 × i16); Surfs; Verts; NumSharedSides; NumZones; 4 bytes;
//! Zones (compact actor + 8 + 8 + 4); Polys; LightMaps (174 bytes + 4 compacts); array at
//! 0xbc (2 bytes, TArray<BYTE>, 12 bytes); FBox; LeafHulls (i32); Leaves (3 compacts + 8);
//! RootOutside.

use crate::coords::{self, Coords, UNIT};
use openxiii_package::{Error, Package, Reader, Result};

pub type V3 = [f32; 3];
pub type Plane = [f32; 4];

/// FScale: scale, shear rate and shear axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub scale: V3,
    pub sheer_rate: f32,
    pub sheer_axis: u8,
}

impl Default for Scale {
    fn default() -> Self {
        Scale { scale: [1.0; 3], sheer_rate: 0.0, sheer_axis: 0 }
    }
}

#[derive(Debug, Clone)]
pub struct BspNode {
    pub plane: Plane,
    pub node_flags: u8,
    pub vert_pool: i32,
    pub surf: i16,
    /// iChild[0] (iBack), iChild[1] (iFront).
    pub child: [i16; 2],
    /// iPlane: next coplanar node.
    pub coplanar: i16,
    pub collision_bound: i32,
    /// FBspNode+0x10: zones touched by the node and its children (one bit per zone number).
    pub zone_mask: u64,
    /// FBspNode+0x38: iRenderBound, box in `bounds` of the node and its children (-1 if none).
    pub render_bound: i16,
    /// FBspNode+0x24: sphere bounding the node's polygon (centre, radius).
    pub sphere: [f32; 4],
    pub zone: [u8; 2],
    pub num_vertices: u8,
    pub leaf: [i16; 2],
}

#[derive(Debug, Clone)]
pub struct BspSurf {
    /// Material (package object reference, 0 without material).
    pub material: i32,
    pub poly_flags: u32,
    /// iBrushPoly: polygon of the source brush (FBspSurf+0x12), -1 if none.
    pub brush_poly: i16,
    /// Source brush of the surface (package object reference, 0 if none).
    pub actor: i32,
    pub plane: Plane,
}

/// Native FBox without its three padding bytes: the coordinates exist even when invalid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawBoundingBox {
    pub min: V3,
    pub max: V3,
    pub valid: u8,
}

impl RawBoundingBox {
    pub fn as_option(self) -> Option<(V3, V3)> {
        (self.valid != 0).then_some((self.min, self.max))
    }
}

#[derive(Debug, Clone)]
pub struct BspModel {
    /// UPrimitive::BoundingBox (min, max), if valid.
    pub bounding_box: Option<(V3, V3)>,
    /// Raw copy kept in sync by read_native/set_bounding_box_raw; do not modify a single field.
    pub raw_bounding_box: RawBoundingBox,
    pub points: Vec<V3>,
    pub nodes: Vec<BspNode>,
    pub surfs: Vec<BspSurf>,
    /// pVertex of each node vertex.
    pub verts: Vec<i16>,
    /// Zones[i].ZoneActor (package object reference, 0 without actor).
    pub zone_actors: Vec<i32>,
    /// FZoneProperties::Connectivity: neighbouring zones of each zone (one bit per zone number).
    pub zone_connectivity: Vec<u64>,
    pub leaf_hulls: Vec<i32>,
    /// UModel+0xC8: Bounds, boxes (min, max) of the nodes (FBspNode::iRenderBound).
    pub bounds: Vec<(V3, V3)>,
    /// UModel+0xE0: FLeaf::iZone of each leaf.
    pub leaf_zones: Vec<i32>,
    pub root_outside: bool,
    /// UModel::Polys (package object reference, 0 if none), the brush's polygons.
    pub polys: i32,
}

/// FPoly (operator<<): Base, Normal, TextureU, TextureV, vertices, PolyFlags…
#[derive(Debug, Clone, PartialEq)]
pub struct Poly {
    pub base: V3,
    pub normal: V3,
    pub vertices: Vec<V3>,
    pub flags: u32,
}

/// UPolys::Serialize: Num, Max then each FPoly (version 100, licensee 58: neither
/// PanU/PanV, nor extra flags).
pub fn read_polys(package: &Package, data: &[u8], index: usize) -> Result<Vec<Poly>> {
    Ok(read_polys_pos(package, data, index)?.0)
}

/// `read_polys`, with the position where reading ends (it must land on the end of the object).
pub fn read_polys_pos(package: &Package, data: &[u8], index: usize) -> Result<(Vec<Poly>, usize)> {
    let object = package.object_data(data, index)?;
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let mut r = Reader::new(&data[..end], object.end);
    let num = r.i32()?;
    let _max = r.i32()?;
    let mut out = Vec::with_capacity(num.max(0) as usize);
    for _ in 0..num.max(0) {
        let n = r.compact()?;
        let base = vec3(&mut r)?;
        let normal = vec3(&mut r)?;
        r.take(24)?;
        let vertices = (0..n.max(0)).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;
        let flags = r.u32()?;
        for _ in 0..5 {
            r.compact()?;
        }
        out.push(Poly { base, normal, vertices, flags });
    }
    Ok((out, r.pos))
}

/// The references UPolys::Serialize sees in file order when the garbage collector walks it: for each
/// FPoly, Actor, ItemName and Material (the first three compact indices after the vertices), as raw indices.
pub fn read_polys_refs(package: &Package, data: &[u8], index: usize) -> Result<Vec<i32>> {
    let object = package.object_data(data, index)?;
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let mut r = Reader::new(&data[..end], object.end);
    let num = r.i32()?;
    let _max = r.i32()?;
    let mut refs = Vec::new();
    for _ in 0..num.max(0) {
        let n = r.compact()?;
        r.take(12 + 12 + 24)?;
        r.take(12 * n.max(0) as usize)?;
        r.u32()?;
        for k in 0..5 {
            let v = r.compact()?;
            if k < 3 {
                refs.push(v);
            }
        }
    }
    Ok(refs)
}

/// Contact of a trace in a model (FCheckResult fields filled by UModel).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BspHit {
    pub location: V3,
    pub normal: V3,
    pub time: f32,
    pub item: i32,
    /// Material of the surface hit (TRACE_Material), package object reference.
    pub material: Option<i32>,
    /// Material written by the trace (even null); false when an extent trace finds no
    /// surface below the start: the result then keeps the Material it had.
    pub material_written: bool,
}

impl BspHit {
    pub fn new(time: f32) -> Self {
        BspHit { location: [0.0; 3], normal: [0.0; 3], time, item: -1, material: None, material_written: false }
    }
}

/// Frames of an owning brush (AActor::ToLocal, vt 0xA0, and ToWorld, vt 0xA4).
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub to_local: Coords,
    pub to_world: Coords,
}

fn vec3(r: &mut Reader) -> Result<V3> {
    Ok([f32::from_bits(r.u32()?), f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)])
}

fn plane(r: &mut Reader) -> Result<Plane> {
    Ok([f32::from_bits(r.u32()?), f32::from_bits(r.u32()?), f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)])
}

fn count(r: &mut Reader) -> Result<usize> {
    let n = r.compact()?;
    if n < 0 {
        return Err(Error::BadIndex(n));
    }
    Ok(n as usize)
}

impl BspModel {
    /// ABrush::PostLoad, a volume's brush outside the editor: Vectors, LightMap, LightBits, Bounds,
    /// light textures and vertex streams emptied, Polys set to NULL (UModel+0x5C), +0x140 set to 0, block +0x148 freed. Only Bounds and Polys
    /// have a field here (the other arrays are not kept by BspModel).
    pub fn abrush_post_load_volume(&mut self) {
        self.bounds.clear();
        self.polys = 0;
    }

    pub fn read(package: &Package, data: &[u8], index: usize) -> Result<Self> {
        let object = package.object_data(data, index)?;
        let export = &package.exports[index];
        let end = export.serial_offset as usize + export.serial_size as usize;
        Self::read_native(&data[..end], object.end)
    }

    /// Native part of a UModel (after the properties), from `start` to the end of `data`.
    pub fn read_native(data: &[u8], start: usize) -> Result<Self> {
        let mut r = Reader::new(data, start);
        // UPrimitive::Serialize.
        let min = vec3(&mut r)?;
        let max = vec3(&mut r)?;
        let valid = r.u8()?;
        r.take(16 + 4 + 4)?;
        // Vectors and Points (non-transactional TArray<FVector>).
        let vectors = count(&mut r)?;
        r.take(vectors * 12)?;
        let points = (0..count(&mut r)?).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;
        // Nodes: TArray<FBspNode> (0x54 bytes),
        // operator<<(FArchive&, FBspNode&), licensee >= 0x29.
        let nodes = (0..count(&mut r)?)
            .map(|_| {
                let plane = plane(&mut r)?;
                let zone_mask = r.u32()? as u64 | (r.u32()? as u64) << 32;
                let node_flags = r.u8()? & 0x1f;
                let vert_pool = r.i32()?;
                let surf = r.u16()? as i16;
                let back = r.u16()? as i16;
                let front = r.u16()? as i16;
                let coplanar = r.u16()? as i16;
                let collision_bound = r.i32()?;
                let render_bound = r.u16()? as i16;
                let sphere = self::plane(&mut r)?;
                let zone = [r.u8()?, r.u8()?];
                let num_vertices = r.u8()?;
                let leaf = [r.u16()? as i16, r.u16()? as i16];
                r.take(4)?;
                Ok(BspNode { plane, node_flags, vert_pool, surf, child: [back, front], coplanar, collision_bound, zone_mask, render_bound, sphere, zone, num_vertices, leaf })
            })
            .collect::<Result<Vec<_>>>()?;
        // Surfs: TArray<FBspSurf> (0x38 bytes),
        // operator<<(FArchive&, FBspSurf&).
        let surfs = (0..count(&mut r)?)
            .map(|_| {
                let material = r.compact()?;
                let poly_flags = r.u32()?;
                r.take(10)?;
                let brush_poly = r.u16()? as i16;
                r.u8()?;
                let actor = r.compact()?;
                let plane = plane(&mut r)?;
                Ok(BspSurf { material, poly_flags, brush_poly, actor, plane })
            })
            .collect::<Result<Vec<_>>>()?;
        // Verts: TArray<FVert>.
        let verts = (0..count(&mut r)?)
            .map(|_| {
                let p = r.u16()? as i16;
                r.u16()?;
                Ok(p)
            })
            .collect::<Result<Vec<_>>>()?;
        let _shared_sides = r.i32()?;
        let num_zones = r.i32()?;
        r.take(4)?;
        let mut zone_connectivity = Vec::new();
        // Zones: FZoneProperties, NumZones times.
        let zone_actors = (0..num_zones.max(0))
            .map(|_| {
                let actor = r.compact()?;
                let lo = r.u32()? as u64;
                let hi = r.u32()? as u64;
                zone_connectivity.push(lo | hi << 32);
                r.take(8 + 4)?;
                Ok(actor)
            })
            .collect::<Result<Vec<_>>>()?;
        let polys = r.compact()?;
        // LightMaps: TArray (0xC0 bytes) (FPlane,
        // FMatrix).
        for _ in 0..count(&mut r)? {
            r.take(4 + 4 + 64 + 64 + 1 + 1 + 36)?;
            for _ in 0..4 {
                r.compact()?;
            }
        }
        // Array at 0xbc: TArray.
        for _ in 0..count(&mut r)? {
            r.take(2)?;
            let n = count(&mut r)?;
            r.take(n + 12)?;
        }
        // Bounds: TArray<FBox>.
        let bounds = (0..count(&mut r)?)
            .map(|_| {
                let min = vec3(&mut r)?;
                let max = vec3(&mut r)?;
                r.u8()?;
                Ok((min, max))
            })
            .collect::<Result<Vec<_>>>()?;
        // LeafHulls: TArray<INT>.
        let leaf_hulls = (0..count(&mut r)?).map(|_| r.i32()).collect::<Result<Vec<_>>>()?;
        // Leaves: TArray<FLeaf>: iZone, iPermeating,
        // iVolumetric, VisibleZones.
        let leaf_zones = (0..count(&mut r)?)
            .map(|_| {
                let zone = r.compact()?;
                r.compact()?;
                r.compact()?;
                r.take(8)?;
                Ok(zone)
            })
            .collect::<Result<Vec<_>>>()?;
        // RootOutside, then Linked and the lighting and render arrays, not read here:
        // (0x20 bytes),
        // lights, TArray<WORD>.
        let root_outside = r.i32()? != 0;
        let model = BspModel { bounding_box: (valid != 0).then_some((min, max)), raw_bounding_box: RawBoundingBox { min, max, valid }, points, nodes, surfs, verts, zone_actors, zone_connectivity, leaf_hulls, bounds, leaf_zones, root_outside, polys };
        model.check_tree()?;
        Ok(model)
    }

    /// The node links read from the file form a tree that the walks can follow: children,
    /// coplanar nodes and render bounds inside their arrays, at most one link to each node,
    /// none to the root, no loop, zone numbers below 64. A damaged or hand-made map that breaks
    /// this is refused when it is loaded instead of crashing or hanging the game later.
    pub fn check_tree(&self) -> Result<()> {
        let bad = |v: i64| openxiii_package::Error::BadIndex(v as i32);
        for n in &self.nodes {
            if n.render_bound != -1 && (n.render_bound < 0 || n.render_bound as usize >= self.bounds.len()) {
                return Err(bad(n.render_bound as i64));
            }
            if let Some(&z) = n.zone.iter().find(|&&z| z >= 64) {
                return Err(bad(z as i64));
            }
        }
        check_node_links(self.nodes.len(), self.nodes.iter().map(|n| [n.child[0], n.child[1], n.coplanar])).map_err(bad)
    }

    /// Keeps the native representation and the projection of valid boxes in sync.
    pub fn set_bounding_box_raw(&mut self, raw: RawBoundingBox) {
        self.raw_bounding_box = raw;
        self.bounding_box = raw.as_option();
    }

    pub fn collision_bounding_box(&self, owner: Option<&Frame>) -> Option<(V3, V3)> {
        self.collision_bounding_box_raw(owner).as_option()
    }

    /// UModel::GetCollisionBoundingBox: a NULL owner copies the raw FBox; otherwise transforms its eight corners.
    pub fn collision_bounding_box_raw(&self, owner: Option<&Frame>) -> RawBoundingBox {
        let raw = self.raw_bounding_box;
        let Some(frame) = owner else { return raw };
        let (min, max) = (raw.min, raw.max);
        // FBox::TransformBy: corner i = (X if i & 4, Y if i & 2, Z if i & 1), the first one
        // sets the box, then FBox +=: Min = Min <= V ? Min : V, Max = Max >= V ? Max : V (unordered picks V)
        // (zero signs and NaN as in the original).
        let mut b: Option<(V3, V3)> = None;
        for i in 0..8 {
            let p = [if i & 4 == 0 { min[0] } else { max[0] }, if i & 2 == 0 { min[1] } else { max[1] }, if i & 1 == 0 { min[2] } else { max[2] }];
            let w = coords::transform_point(p, &frame.to_world);
            b = Some(match b {
                None => (w, w),
                Some((lo, hi)) => ([0, 1, 2].map(|k| if lo[k] <= w[k] { lo[k] } else { w[k] }), [0, 1, 2].map(|k| if hi[k] >= w[k] { hi[k] } else { w[k] })),
            });
        }
        let (min, max) = b.expect("eight corners always visited");
        RawBoundingBox { min, max, valid: 1 }
    }

    fn node(&self, i: i32) -> Option<&BspNode> {
        usize::try_from(i).ok().and_then(|i| self.nodes.get(i))
    }

    /// UModel::PointCheck: false if the box `extent` centred on `location` touches
    /// the inside of the model. Without extent, the point is brought into model space and follows
    /// the planes down to a leaf; with extent, FBoxPointCheck.
    /// UModel::FastLineCheck (traversal): true if the segment only crosses
    /// empty space. At each node, the sides of Start and End by the sign of their plane distances (sign
    /// bit, -0 counts as behind); an NF_NotCsg node (bit 0 of NodeFlags) keeps its parent's
    /// "outside" state; the part beyond the plane is walked first.
    pub fn fast_line_check(&self, end: V3, start: V3) -> bool {
        if self.nodes.is_empty() {
            return self.root_outside;
        }
        self.fast_line_node(0, end, start, self.root_outside)
    }

    fn fast_line_node(&self, mut i_node: i32, mut end: V3, start: V3, mut outside: bool) -> bool {
        let start = start;
        while let Some(node) = self.node(i_node) {
            let n = node.plane;
            // (Z + Y) + X.
            let d0 = ((start[2] * n[2] + start[1] * n[1]) + start[0] * n[0]) - n[3];
            let d1 = ((end[2] * n[2] + end[1] * n[1]) + end[0] * n[0]) - n[3];
            let not_csg = node.node_flags & 1 != 0;
            let g1 = !d0.is_sign_negative();
            let g2 = !d1.is_sign_negative();
            if g1 != g2 {
                let alpha = d0 / (d0 - d1);
                let middle = [(end[0] - start[0]) * alpha + start[0], (end[1] - start[1]) * alpha + start[1], (end[2] - start[2]) * alpha + start[2]];
                let child_outside = if not_csg { outside } else { g2 };
                if !self.fast_line_node(node.child[g2 as usize] as i32, middle, end, child_outside) {
                    return false;
                }
                end = middle;
            }
            outside = if not_csg { outside } else { g1 };
            i_node = node.child[g1 as usize] as i32;
        }
        outside
    }

    pub fn point_check(&self, hit: &mut BspHit, owner: Option<&Frame>, location: V3, extent: V3, extra_node_flags: u32) -> bool {
        hit.normal = [0.0; 3];
        hit.location = location;
        hit.time = 0.0;
        let mut outside = self.root_outside;
        if self.nodes.is_empty() {
            return outside;
        }
        if extent[0] == 0.0 && extent[1] == 0.0 && extent[2] == 0.0 {
            let p = coords::transform_point(location, &owner.map_or(UNIT, |o| o.to_local));
            let mut i_node = 0i32;
            while let Some(node) = self.node(i_node) {
                let n = node.plane;
                let open = node.num_vertices == 0 || node.node_flags & 0x21 != 0;
                // (Y + Z) + X.
                let side = if ((p[1] * n[1] + p[2] * n[2]) + p[0] * n[0]) - n[3] <= 0.0 {
                    outside = outside && open;
                    0
                } else {
                    outside = outside || !open;
                    1
                };
                hit.item = side + i_node * 2;
                i_node = node.child[side as usize] as i32;
            }
            return outside;
        }
        // FPointCheck: Location, BestDist = 100,000.
        let mut check = Check::new(self, owner, extent, extra_node_flags, *hit);
        check.point = location;
        check.best_dist = 100000.0;
        let result = check.box_point_check(0, 0, outside);
        *hit = check.hit;
        result
    }

    /// UModel::LineCheck: false if the path from `start` to `end` (box `extent`)
    /// touches the model, with Hit.Time and Hit.Location pulled back as the engine does.
    #[allow(clippy::too_many_arguments)]
    pub fn line_check(&self, hit: &mut BspHit, owner: Option<&Frame>, end: V3, start: V3, extent: V3, extra_node_flags: u32, trace_flags: u32) -> bool {
        if self.nodes.is_empty() {
            return self.root_outside;
        }
        let d = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
        if extent[0] == 0.0 && extent[1] == 0.0 && extent[2] == 0.0 {
            // Reset on every trace, set as soon as an outside leaf is seen.
            let mut been_outside = false;
            let to_world = owner.map(|o| o.to_world);
            if self.line_check_zero(hit, to_world.as_ref(), 0, 0, end, start, self.root_outside, extra_node_flags, &mut been_outside) {
                return true;
            }
            // (X + Y) + Z.
            let len2 = (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2];
            hit.time = (((hit.location[0] - start[0]) * d[0] + (hit.location[1] - start[1]) * d[1]) + (hit.location[2] - start[2]) * d[2]) / len2;
            let len = (len2 as f64).sqrt() as f32;
            let t = hit.time - 0.5 / len;
            hit.time = if t >= 0.0 { t.min(1.0) } else { 0.0 };
            hit.location = [d[0] * hit.time + start[0], start[1] + d[1] * hit.time, start[2] + d[2] * hit.time];
            hit.item = self.coplanar_containing(hit.item, hit.location);
            let surf = self.node(hit.item).and_then(|n| usize::try_from(n.surf).ok()).and_then(|s| self.surfs.get(s));
            if extra_node_flags & 0x100 != 0 && hit.item != -1 && surf.is_some_and(|s| s.poly_flags as u8 & 0x81 != 0) {
                return true;
            }
            if trace_flags & 0x1000 != 0 {
                hit.material = if hit.item == -1 { None } else { surf.map(|s| s.material) };
                hit.material_written = true;
            }
            if let Some(o) = owner {
                hit.normal = coords::transform_vector(hit.normal, &o.to_world);
            }
            // (Z + Y) + X.
            if (-d[2] * hit.normal[2] + -d[1] * hit.normal[1]) + -d[0] * hit.normal[0] < 0.0 {
                hit.normal = [-hit.normal[0], -hit.normal[1], -hit.normal[2]];
            }
            return false;
        }
        hit.time = 2.0;
        // FLineCheck: End, Start, Dist, TraceFlags.
        let mut check = Check::new(self, owner, extent, extra_node_flags, *hit);
        check.end = end;
        check.start = start;
        // (X² + Y²) + Z² (FLineCheck), appSqrt.
        check.dist = (((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]) as f64).sqrt() as f32;
        check.trace_flags = trace_flags;
        check.box_line_check(0, 0, self.root_outside);
        *hit = check.hit;
        if check.b_hit {
            let inv = 1.0 / check.dist;
            let mut pull = inv * 0.1;
            if pull <= 0.1 {
                pull = 4.0 * inv;
                if 0.1 < pull {
                    pull = 0.1;
                }
            }
            let t = hit.time - pull;
            hit.time = if t >= 0.0 { t.min(1.0) } else { 0.0 };
            hit.location = [d[0] * hit.time + start[0], d[1] * hit.time + start[1], d[2] * hit.time + start[2]];
            if hit.time != 1.0 {
                return false;
            }
        }
        true
    }

    /// Zero-extent trace through the planes: true without contact; otherwise Hit.Location
    /// at the entry point, Hit.Normal the plane of the last node crossed (model space), Hit.Item
    /// that node. Bit 0x10 of ExtraNodeFlags only accepts a contact after an outside leaf.
    #[allow(clippy::too_many_arguments)]
    fn line_check_zero(
        &self,
        hit: &mut BspHit,
        coords: Option<&Coords>,
        mut i_hit: i32,
        mut i_node: i32,
        end: V3,
        mut start: V3,
        mut outside: bool,
        extra: u32,
        been_outside: &mut bool,
    ) -> bool {
        let csg_mask = (extra as u8 & 0xef) | 0x21;
        let cross_mask = extra as u8 | 0x21;
        loop {
            let Some(node) = self.node(i_node) else {
                if outside {
                    *been_outside = true;
                    return true;
                }
                if !*been_outside && extra & 0x10 != 0 {
                    return true;
                }
                hit.location = start;
                let n = self.node(i_hit).map_or([0.0; 4], |n| n.plane);
                hit.normal = [n[0], n[1], n[2]];
                hit.item = i_hit;
                return false;
            };
            let (d_start, d_end) = match coords {
                // (X + Y) + Z.
                None => {
                    let n = node.plane;
                    (((start[0] * n[0] + start[1] * n[1]) + start[2] * n[2]) - n[3], ((end[0] * n[0] + end[1] * n[1]) + end[2] * n[2]) - n[3])
                }
                // TransformPlaneByOrtho inlined, twice: for Start W − ((O.Y·N.Y + O.X·N.X) +
                // O.Z·N.Z), for End W − ((O.Z·N.Z + O.Y·N.Y) + O.X·N.X);
                // then (Y + Z) + X.
                Some(c) => {
                    let n = coords::transform_vector([node.plane[0], node.plane[1], node.plane[2]], c);
                    let o = coords::transform_vector(c.origin, c);
                    let w_start = node.plane[3] - ((o[1] * n[1] + o[0] * n[0]) + o[2] * n[2]);
                    let w_end = node.plane[3] - ((o[2] * n[2] + o[1] * n[1]) + o[0] * n[0]);
                    (((start[1] * n[1] + start[2] * n[2]) + n[0] * start[0]) - w_start, ((end[1] * n[1] + end[2] * n[2]) + n[0] * end[0]) - w_end)
                }
            };
            if d_start > -0.001 && d_end > -0.001 {
                if !(node.num_vertices == 0 || csg_mask & node.node_flags != 0) {
                    outside = true;
                }
                i_node = node.child[1] as i32;
                continue;
            }
            if d_start < 0.001 && d_end < 0.001 {
                let csg = !(node.num_vertices == 0 || csg_mask & node.node_flags != 0);
                outside = outside && !csg;
                i_node = node.child[0] as i32;
                continue;
            }
            // The path crosses the plane: first the part on the `start` side.
            let t = d_start / (d_end - d_start);
            let middle = [(start[0] - end[0]) * t + start[0], (start[1] - end[1]) * t + start[1], (start[2] - end[2]) * t + start[2]];
            let open = node.num_vertices == 0 || node.node_flags & cross_mask != 0;
            let front_first = d_start > 0.0;
            let first_outside = if front_first { outside || !open } else { outside && open };
            if !self.line_check_zero(hit, coords, i_hit, node.child[front_first as usize] as i32, middle, start, first_outside, extra, been_outside) {
                return false;
            }
            outside = if front_first { outside && open } else { outside || !open };
            i_hit = i_node;
            i_node = node.child[!front_first as usize] as i32;
            start = middle;
        }
    }

    /// Among `i_node` and its coplanar nodes (iPlane), the one whose polygon
    /// contains `p` (sides seen from the surface normal), otherwise −1.
    fn coplanar_containing(&self, mut i_node: i32, p: V3) -> i32 {
        while let Some(node) = self.node(i_node) {
            if node.num_vertices != 0 {
                let n = self.surfs.get(node.surf as usize).map_or([0.0; 4], |s| s.plane);
                let vert = |k: i32| {
                    let v = usize::try_from(node.vert_pool + k).ok().and_then(|i| self.verts.get(i)).copied().unwrap_or(0);
                    self.points.get(v as usize).copied().unwrap_or([0.0; 3])
                };
                let mut prev = vert(node.num_vertices as i32 - 1);
                let mut side = 0.0f32;
                let mut inside = true;
                for k in 0..node.num_vertices as i32 {
                    let v = vert(k);
                    let e = [v[0] - prev[0], v[1] - prev[1], v[2] - prev[2]];
                    let c = [n[1] * e[2] - n[2] * e[1], n[2] * e[0] - e[2] * n[0], e[1] * n[0] - n[1] * e[0]];
                    let d = ((c[2] * p[2] + p[1] * c[1]) + c[0] * p[0]) - ((c[1] * v[1] + c[2] * v[2]) + c[0] * v[0]);
                    if (d < 0.0 && 0.0 < side) || (0.0 < d && side < 0.0) {
                        inside = false;
                        break;
                    }
                    prev = v;
                    side = d;
                }
                if inside {
                    return i_node;
                }
            }
            i_node = node.coplanar as i32;
        }
        -1
    }

    /// UModel::BoxLeaves: leaves touched by the box (min, max),
    /// in traversal order (back then front of each node).
    pub fn box_leaves(&self, min: V3, max: V3) -> Vec<i32> {
        let mut out = Vec::new();
        self.box_leaves_into(min, max, &mut out);
        out
    }

    /// `box_leaves` into `out` (cleared first), to reuse a buffer: same list.
    pub fn box_leaves_into(&self, min: V3, max: V3, out: &mut Vec<i32>) {
        out.clear();
        if self.nodes.is_empty() {
            return;
        }
        let extent = [0, 1, 2].map(|k| (max[k] - min[k]) * 0.5);
        let center = [0, 1, 2].map(|k| extent[k] + min[k]);
        self.box_leaves_node(0, center, extent, out);
    }

    fn box_leaves_node(&self, mut i: usize, center: V3, extent: V3, out: &mut Vec<i32>) {
        loop {
            let n = &self.nodes[i];
            let p = n.plane;
            // (|Z| + |Y|) + |X| and (X + Z) + Y.
            let push = ((p[2] * extent[2]).abs() + (extent[1] * p[1]).abs()) + (p[0] * extent[0]).abs();
            let dist = ((p[0] * center[0] + p[2] * center[2]) + p[1] * center[1]) - p[3];
            if dist < push {
                if n.child[0] == -1 {
                    if n.leaf[0] != -1 {
                        out.push(n.leaf[0] as i32);
                    }
                } else {
                    self.box_leaves_node(n.child[0] as usize, center, extent, out);
                }
            }
            if dist <= -push {
                return;
            }
            if n.child[1] == -1 {
                if n.leaf[1] != -1 {
                    out.push(n.leaf[1] as i32);
                }
                return;
            }
            i = n.child[1] as usize;
        }
    }

    /// UModel::PointRegion: (Zones[n].ZoneActor, iLeaf, n) of the point; without a BSP,
    /// (0, −1, 0). A null ZoneActor designates the LevelInfo.
    pub fn point_region(&self, p: V3) -> (i32, i32, u8) {
        if self.nodes.is_empty() {
            return (0, -1, 0);
        }
        let mut outside = self.root_outside;
        let (mut parent, mut side) = (0i32, 0usize);
        let mut i_node = 0i32;
        while let Some(node) = self.node(i_node) {
            parent = i_node;
            let n = node.plane;
            let open = node.num_vertices == 0 || node.node_flags & 0x21 != 0;
            // (Y + Z) + X.
            if ((p[1] * n[1] + p[2] * n[2]) + p[0] * n[0]) - n[3] < 0.0 {
                side = 0;
                outside = outside && open;
            } else {
                side = 1;
                outside = outside || !open;
            }
            i_node = node.child[side] as i32;
        }
        let node = &self.nodes[parent as usize];
        let zone = if self.zone_actors.is_empty() { 0 } else { node.zone[side] };
        let actor = self.zone_actors.get(zone as usize).copied().unwrap_or(0);
        (actor, node.leaf[side] as i32, zone)
    }
}

// ---- Brush frames ----

/// Links of a BSP read from a file (back, front, coplanar of each node): each one -1 or a node
/// other than the root, at most one link to each node, and no loop. Err: the faulty index.
pub(crate) fn check_node_links(count: usize, links: impl Iterator<Item = [i16; 3]>) -> std::result::Result<(), i64> {
    let mut parent: Vec<Option<usize>> = vec![None; count];
    for (i, l) in links.enumerate() {
        for link in l {
            if link == -1 {
                continue;
            }
            if link <= 0 || link as usize >= count || parent[link as usize].replace(i).is_some() {
                return Err(link as i64);
            }
        }
    }
    // With one parent at most, a loop is a chain of parents that never reaches a node without one.
    // 0: not seen, 1: on the chain being followed, 2: reaches a root.
    let mut state = vec![0u8; count];
    for start in 0..count {
        let mut chain = Vec::new();
        let mut i = start;
        loop {
            match state[i] {
                2 => break,
                1 => return Err(i as i64),
                _ => {}
            }
            state[i] = 1;
            chain.push(i);
            match parent[i] {
                Some(p) => i = p,
                None => break,
            }
        }
        for k in chain {
            state[k] = 2;
        }
    }
    Ok(())
}

/// FSheerSnap.
pub fn sheer_snap(s: f32) -> f32 {
    if s < -0.65 {
        s + 0.15
    } else if 0.65 < s {
        s - 0.15
    } else if s < -0.55 {
        -0.5
    } else if 0.55 < s {
        0.5
    } else if s < -0.05 {
        s + 0.05
    } else if 0.05 < s {
        s - 0.05
    } else {
        0.0
    }
}

/// Shear frame of the FScale operations: UnitCoords with one term (SheerAxis 1 to 6:
/// X.Y, X.Z, Y.X, Y.Z, Z.X, Z.Y, jump tables).
fn sheer_coords(s: &Scale, sheer: f32) -> Coords {
    let mut t = UNIT;
    match s.sheer_axis {
        1 => t.x[1] = sheer,
        2 => t.x[2] = sheer,
        3 => t.y[0] = sheer,
        4 => t.y[2] = sheer,
        5 => t.z[0] = sheer,
        6 => t.z[1] = sheer,
        _ => {}
    }
    t
}

/// FCoords * FScale (shear first, then each axis multiplied component
/// by component, the origin divided).
pub fn mul_scale(c: &Coords, s: &Scale) -> Coords {
    let mut r = c.mul(&sheer_coords(s, sheer_snap(s.sheer_rate)));
    for axis in [&mut r.x, &mut r.y, &mut r.z] {
        axis[0] *= s.scale[0];
        axis[1] *= s.scale[1];
        axis[2] *= s.scale[2];
    }
    r.origin = [r.origin[0] / s.scale[0], r.origin[1] / s.scale[1], r.origin[2] / s.scale[2]];
    r
}

/// FCoords / FScale (each axis divided by the component of its rank, the origin
/// multiplied, then the inverse shear).
pub fn div_scale(c: &Coords, s: &Scale) -> Coords {
    let inv = [1.0 / s.scale[0], 1.0 / s.scale[1], 1.0 / s.scale[2]];
    let r = Coords {
        origin: [s.scale[0] * c.origin[0], c.origin[1] * s.scale[1], c.origin[2] * s.scale[2]],
        x: c.x.map(|v| inv[0] * v),
        y: c.y.map(|v| inv[1] * v),
        z: c.z.map(|v| inv[2] * v),
    };
    r.mul(&sheer_coords(s, -sheer_snap(s.sheer_rate)))
}

/// AActor::ToLocal (vtable +0xA0): Unit / Rotation / Location. Frame of an actor
/// that is not a Brush (in XIII, Mover inherits from Actor): neither PrePivot nor scale, and no
/// unit scale operation, which would change the sign of zeros.
pub fn actor_to_local(location: V3, rotation: [i32; 3]) -> Coords {
    let mut c = coords::div_rotator(&UNIT, rotation);
    c.origin = [location[0] + c.origin[0], location[1] + c.origin[1], location[2] + c.origin[2]];
    c
}

/// AActor::ToWorld (vtable +0xA4): Unit * Location (Origin − Location) * Rotation.
pub fn actor_to_world(location: V3, rotation: [i32; 3]) -> Coords {
    let mut c = UNIT;
    c.origin = [c.origin[0] - location[0], c.origin[1] - location[1], c.origin[2] - location[2]];
    coords::mul_rotator(&c, rotation)
}

/// ABrush::ToLocal: Unit / −PrePivot / MainScale / Rotation / PostScale / Location.
pub fn brush_to_local(location: V3, rotation: [i32; 3], pre_pivot: V3, main_scale: &Scale, post_scale: &Scale) -> Coords {
    let mut c = UNIT;
    c.origin = [c.origin[0] - pre_pivot[0], c.origin[1] - pre_pivot[1], c.origin[2] - pre_pivot[2]];
    c = div_scale(&c, main_scale);
    c = coords::div_rotator(&c, rotation);
    c = div_scale(&c, post_scale);
    c.origin = [c.origin[0] + location[0], c.origin[1] + location[1], c.origin[2] + location[2]];
    c
}

/// ABrush::ToWorld: Unit * Location * PostScale * Rotation * MainScale * −PrePivot.
pub fn brush_to_world(location: V3, rotation: [i32; 3], pre_pivot: V3, main_scale: &Scale, post_scale: &Scale) -> Coords {
    let mut c = UNIT;
    c.origin = [c.origin[0] - location[0], c.origin[1] - location[1], c.origin[2] - location[2]];
    c = mul_scale(&c, post_scale);
    c = coords::mul_rotator(&c, rotation);
    c = mul_scale(&c, main_scale);
    c.origin = [c.origin[0] + pre_pivot[0], c.origin[1] + pre_pivot[1], c.origin[2] + pre_pivot[2]];
    c
}

/// FPlane::TransformPlaneByOrtho.
fn transform_plane_by_ortho(p: Plane, c: &Coords) -> Plane {
    let n = coords::transform_vector([p[0], p[1], p[2]], c);
    let o = coords::transform_vector(c.origin, c);
    [n[0], n[1], n[2], p[3] - ((o[0] * n[0] + o[1] * n[1]) + o[2] * n[2])]
}

/// appRSqrt: fsqrt then 1 / root, each rounded to 24 bits (the game's FPU).
fn rsqrt(x: f32) -> f32 {
    1.0 / x.sqrt()
}

/// FVector::SafeNormal (zero below 1e-8).
fn safe_normal(v: V3) -> V3 {
    let sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if (sq as f64) < 1e-8 {
        return [0.0; 3];
    }
    let s = rsqrt(sq);
    [s * v[0], s * v[1], s * v[2]]
}

/// FPlane::TransformPlaneBy (normal transformed then renormalised, plane point
/// transformed).
fn transform_plane_by(p: Plane, c: &Coords) -> Plane {
    let n = coords::transform_vector([p[0], p[1], p[2]], c);
    let base = coords::transform_point([p[3] * p[0], p[3] * p[1], p[3] * p[2]], c);
    let n = safe_normal(n);
    [n[0], n[1], n[2], (base[0] * n[0] + base[2] * n[2]) + base[1] * n[1]]
}

/// Line of intersection of two planes (point, direction A × B); zero for
/// parallel planes (|A × B|² < 1e-6).
fn plane_intersection(a: Plane, b: Plane) -> (V3, V3) {
    let dir = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - b[2] * a[0], a[0] * b[1] - a[1] * b[0]];
    // (X² + Y²) + Z².
    let len2 = (dir[0] * dir[0] + dir[1] * dir[1]) + dir[2] * dir[2];
    // (0x358637BE, slightly more than 1e-6).
    if len2 < f32::from_bits(0x3586_37be) {
        return ([0.0; 3], [0.0; 3]);
    }
    let inv = 1.0 / len2;
    let p = [
        ((dir[2] * b[1] - dir[1] * b[2]) * a[3] + (dir[1] * a[2] - dir[2] * a[1]) * b[3]) * inv,
        ((dir[0] * b[2] - dir[2] * b[0]) * a[3] + (dir[2] * a[0] - dir[0] * a[2]) * b[3]) * inv,
        ((dir[1] * b[0] - dir[0] * b[1]) * a[3] + (a[1] * dir[0] - dir[1] * a[0]) * b[3]) * inv,
    ];
    // The returned direction is normalised (FVector::Normalize).
    let mut dir = dir;
    coords::normalize(&mut dir);
    (p, dir)
}

/// Edge bevel: normal `v` normalised (appRSqrt), facing plane `pi`,
/// through `point`.
/// Machine-code order: length (X² + Y²) + Z², direction
/// (N.Y·P.Y + N.Z·P.Z) + N.X·P.X, W = (Q.Y·N.Y + Q.Z·N.Z) + Q.X·N.X.
fn bevel(v: V3, pi: Plane, point: V3) -> Plane {
    let s = rsqrt((v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]);
    let mut n = [s * v[0], s * v[1], s * v[2]];
    if (n[1] * pi[1] + n[2] * pi[2]) + n[0] * pi[0] < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    [n[0], n[1], n[2], (point[1] * n[1] + point[2] * n[2]) + point[0] * n[0]]
}

/// Arrays of a leaf's hull (at most 0x40 planes) kept from one Check to the next: returned at the
/// end of each Check, cleared and reused by the next (no allocation per trace).
type BuffersWrapper = (Vec<Plane>, Vec<u32>, Vec<i32>);
thread_local! {
    static BUFFERS_WRAPPER: std::cell::RefCell<Vec<BuffersWrapper>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// State of the extent checks (FCollisionCheck, FBoxPointCheck and FBoxLineCheck).
struct Check<'a> {
    model: &'a BspModel,
    hit: BspHit,
    owner: bool,
    /// Owner->ToWorld() (vt 0xA4) or UnitCoords.
    coords: Coords,
    extent: V3,
    extra: u32,
    hull_planes: Vec<Plane>,
    hull_flags: Vec<u32>,
    hull_nodes: Vec<i32>,
    /// Frame origin passed through the frame itself (FBoxPointCheck, the same at every node).
    origin_w: V3,
    box_min: V3,
    box_max: V3,
    // FBoxPointCheck.
    point: V3,
    best_dist: f32,
    // FBoxLineCheck.
    end: V3,
    start: V3,
    dist: f32,
    trace_flags: u32,
    b_hit: bool,
    t0: f32,
    t1: f32,
    hit_normal: V3,
}

impl Drop for Check<'_> {
    fn drop(&mut self) {
        let mut t = (std::mem::take(&mut self.hull_planes), std::mem::take(&mut self.hull_flags), std::mem::take(&mut self.hull_nodes));
        t.0.clear();
        t.1.clear();
        t.2.clear();
        BUFFERS_WRAPPER.with(|p| p.borrow_mut().push(t));
    }
}

impl<'a> Check<'a> {
    fn new(model: &'a BspModel, owner: Option<&Frame>, extent: V3, extra: u32, hit: BspHit) -> Self {
        let (hull_planes, hull_flags, hull_nodes) = BUFFERS_WRAPPER
            .with(|t| t.borrow_mut().pop())
            .unwrap_or_else(|| (Vec::with_capacity(64), Vec::with_capacity(64), Vec::with_capacity(64)));
        Check {
            model,
            hit,
            owner: owner.is_some(),
            coords: owner.map_or(UNIT, |o| o.to_world),
            extent,
            extra,
            hull_planes,
            hull_flags,
            hull_nodes,
            origin_w: owner.map_or([0.0; 3], |o| coords::transform_vector(o.to_world.origin, &o.to_world)),
            box_min: [0.0; 3],
            box_max: [0.0; 3],
            point: [0.0; 3],
            best_dist: 0.0,
            end: [0.0; 3],
            start: [0.0; 3],
            dist: 0.0,
            trace_flags: 0,
            b_hit: false,
            t0: 0.0,
            t1: 0.0,
            hit_normal: [0.0; 3],
        }
    }

    /// Planes of the leaf's convex hull (LeafHulls from
    /// iCollisionBound, at most 64, bit 0x40000000 = flipped plane), signs of their normals and
    /// the box that follows the final −1.
    fn setup_hulls(&mut self, parent: &BspNode) {
        let m = self.model;
        self.hull_planes.clear();
        self.hull_flags.clear();
        self.hull_nodes.clear();
        let start = parent.collision_bound as usize;
        while let Some(&v) = m.leaf_hulls.get(start + self.hull_planes.len()) {
            if v == -1 || self.hull_planes.len() >= 0x40 {
                break;
            }
            let mut p = m.node((v as u32 & 0xbfff_ffff) as i32).map_or([0.0; 4], |n| n.plane);
            if self.owner {
                p = transform_plane_by_ortho(p, &self.coords);
            }
            if v as u32 & 0x4000_0000 != 0 {
                p = [-p[0], -p[1], -p[2], -p[3]];
            }
            let sign = |x: f32, neg: u32, pos: u32| match x.partial_cmp(&0.0) {
                Some(std::cmp::Ordering::Greater) => pos,
                Some(std::cmp::Ordering::Equal) => 0,
                _ => neg,
            };
            self.hull_flags.push(sign(p[0], 1, 2) | sign(p[1], 4, 8) | sign(p[2], 0x10, 0x20));
            self.hull_planes.push(p);
            self.hull_nodes.push(v);
        }
        let b = start + self.hull_planes.len() + 1;
        let f = |k: usize| m.leaf_hulls.get(b + k).map_or(0.0, |&x| f32::from_bits(x as u32));
        self.box_min = [f(0), f(1), f(2)];
        self.box_max = [f(3), f(4), f(5)];
    }

    /// Bevels of the leaf's box (model without owner) then bevels of the edges
    /// between two hull planes with opposite signs on an axis, in engine order.
    /// Each bevel is passed to `test` as soon as it is computed, without an array; stops (false) at
    /// the first rejected one, like the engine's loop over the whole array.
    fn bevels_each(&mut self, mut test: impl FnMut(&mut Self, Plane) -> bool) -> bool {
        let (planes, flags) = (std::mem::take(&mut self.hull_planes), std::mem::take(&mut self.hull_flags));
        let ok = self.bevels_each_in(&planes, &flags, &mut test);
        (self.hull_planes, self.hull_flags) = (planes, flags);
        ok
    }

    fn bevels_each_in(&mut self, hull_planes: &[Plane], hull_flags: &[u32], test: &mut impl FnMut(&mut Self, Plane) -> bool) -> bool {
        macro_rules! out_push {
            ($p:expr) => {
                let p = $p;
                if !test(self, p) {
                    return false;
                }
            };
        }
        if !self.owner {
            let (lo, hi) = (self.box_min, self.box_max);
            let d = |a: f64, b: f32| (a - b as f64) as f32;
            out_push!([0.0, 0.0, -1.0, d(0.1, lo[2])]);
            out_push!([0.0, 0.0, 1.0, (hi[2] as f64 + 0.1) as f32]);
            out_push!([-1.0, 0.0, 0.0, d(0.1, lo[0])]);
            out_push!([1.0, 0.0, 0.0, (hi[0] as f64 - 0.1) as f32]);
            out_push!([0.0, -1.0, 0.0, d(0.1, lo[1])]);
            out_push!([0.0, 1.0, 0.0, (hi[1] as f64 - 0.1) as f32]);
        }
        for i in 0..hull_planes.len() {
            for j in 0..i {
                let f = hull_flags[i] | hull_flags[j];
                let (pi, pj) = (hull_planes[i], hull_planes[j]);
                if f & 3 == 3 {
                    // (Z + Y) + X of the products of the two edges.
                    let c = ((pj[1] - pj[0] * 0.0) * (pi[1] - pi[0] * 0.0) + (pj[0] * 0.0 - pj[2]) * (pi[0] * 0.0 - pi[2]))
                        + (pj[2] * 0.0 - pj[1] * 0.0) * (pi[2] * 0.0 - pi[1] * 0.0);
                    if 0.001 < c {
                        let (point, dir) = plane_intersection(pi, pj);
                        out_push!(bevel([dir[2] * 0.0 - dir[1] * 0.0, dir[0] * 0.0 - dir[2], dir[1] - dir[0] * 0.0], pi, point));
                    }
                }
                if f & 0xc == 0xc {
                    let c = ((pj[1] * 0.0 - pj[0]) * (pi[1] * 0.0 - pi[0]) + (pj[0] * 0.0 - pj[2] * 0.0) * (pi[0] * 0.0 - pi[2] * 0.0))
                        + (pj[2] - pj[1] * 0.0) * (pi[2] - pi[1] * 0.0);
                    if 0.001 < c {
                        let (point, dir) = plane_intersection(pi, pj);
                        out_push!(bevel([dir[2] - dir[1] * 0.0, dir[0] * 0.0 - dir[2] * 0.0, dir[1] * 0.0 - dir[0]], pi, point));
                    }
                }
                if f & 0x30 == 0x30 {
                    let c = ((pj[1] * 0.0 - pj[0] * 0.0) * (pi[1] * 0.0 - pi[0] * 0.0) + (pj[0] - pj[2] * 0.0) * (pi[0] - pi[2] * 0.0))
                        + (pj[2] * 0.0 - pj[1]) * (pi[2] * 0.0 - pi[1]);
                    if 0.001 < c {
                        let (point, dir) = plane_intersection(pi, pj);
                        out_push!(bevel([dir[2] * 0.0 - dir[1], dir[0] - dir[2] * 0.0, dir[1] * 0.0 - dir[0] * 0.0], pi, point));
                    }
                }
            }
        }
        true
    }

    /// Does the box touch the inside of the hull plane? Keeps the closest
    /// plane (BestDist) as the contact, the box pushed back by 1.02 × the penetration.
    fn point_check_hull_side(&mut self, p: Plane, item: i32) -> bool {
        let e = self.extent;
        let push_out = (e[2] * p[2]).abs() + (e[1] * p[1]).abs() + (p[0] * e[0]).abs();
        let q = self.point;
        let dist = ((q[1] * p[1] + q[2] * p[2]) + q[0] * p[0]) - p[3];
        if 0.0 < dist && dist < self.best_dist {
            if push_out <= dist {
                return false;
            }
            self.best_dist = dist;
            let k = push_out - dist;
            self.hit.location = [p[0] * 1.02 * k + q[0], k * (p[1] * 1.02) + q[1], (p[2] * 1.02) * k + q[2]];
            self.hit.normal = [p[0], p[1], p[2]];
            self.hit.item = item;
            self.hit.time = 0.0;
        }
        dist < push_out
    }

    /// FBoxPointCheck, true if the box is outside the model below `i_node`.
    fn box_point_check(&mut self, mut i_parent: i32, mut i_node: i32, mut outside: bool) -> bool {
        let m = self.model;
        let mut result = true;
        while let Some(node) = m.node(i_node) {
            let p = if self.owner {
                let n = coords::transform_vector([node.plane[0], node.plane[1], node.plane[2]], &self.coords);
                let o = self.origin_w;
                [n[0], n[1], n[2], node.plane[3] - ((o[1] * n[1] + o[2] * n[2]) + o[0] * n[0])]
            } else {
                node.plane
            };
            let e = self.extent;
            let push_out = (p[2] * (e[2] * 1.1)).abs() + (p[1] * (e[1] * 1.1)).abs() + (e[0] * 1.1 * p[0]).abs();
            let q = self.point;
            let dist = ((p[2] * q[2] + p[1] * q[1]) + p[0] * q[0]) - p[3];
            let csg = node.num_vertices != 0 && ((self.extra as u8 | 0x21) & node.node_flags) == 0;
            if -push_out < dist && !self.box_point_check(i_node, node.child[1] as i32, outside || csg) {
                result = false;
            }
            outside = outside && !csg;
            i_parent = i_node;
            i_node = node.child[0] as i32;
            if push_out < dist {
                return result;
            }
        }
        if outside {
            return result;
        }
        let Some(parent) = m.node(i_parent) else { return result };
        if parent.collision_bound == -1 {
            return result;
        }
        self.setup_hulls(parent);
        for i in 0..self.hull_planes.len() {
            let (p, item) = (self.hull_planes[i], (self.hull_nodes[i] as u32 & 0xbfff_ffff) as i32);
            if !self.point_check_hull_side(p, item) {
                return result;
            }
        }
        if !self.bevels_each(|c, p| c.point_check_hull_side(p, -1)) {
            return result;
        }
        false
    }

    /// Clipping of the path by a hull plane (T0 at entry, T1 at exit).
    fn clip_to(&mut self, p: Plane) -> bool {
        let e = self.extent;
        let push_out = (e[2] * p[2]).abs() + (e[1] * p[1]).abs() + (p[0] * e[0]).abs();
        let (s, end_pt) = (self.start, self.end);
        let d0 = ((s[1] * p[1] + s[2] * p[2]) + s[0] * p[0]) - p[3];
        let d1 = ((end_pt[1] * p[1] + end_pt[2] * p[2]) + end_pt[0] * p[0]) - p[3];
        let mut t = d0 - push_out;
        if d1 < d0 && -push_out <= t && t < 0.0 {
            t = 0.0;
        }
        let denom = d0 - d1;
        let t = t / denom;
        if -1e-5 <= denom {
            if denom <= 1e-5 {
                if push_out < d0 && push_out < d1 {
                    return false;
                }
            } else if self.t0 < t {
                self.t0 = t;
                self.hit_normal = [p[0], p[1], p[2]];
            }
        } else if t < self.t1 {
            self.t1 = t;
        }
        self.t1 > self.t0
    }

    /// FBoxLineCheck, descends both sides touched by the swept box then
    /// clips the path by the hull of each solid leaf.
    fn box_line_check(&mut self, mut i_parent: i32, mut i_node: i32, mut outside: bool) {
        let m = self.model;
        loop {
            let Some(node) = m.node(i_node) else {
                self.leaf_line_check(i_parent, outside);
                return;
            };
            let p = if self.owner { transform_plane_by(node.plane, &self.coords) } else { node.plane };
            let (s, end_pt) = (self.start, self.end);
            let d0 = ((p[0] * s[0] + p[1] * s[1]) + p[2] * s[2]) - p[3];
            let d1 = ((p[0] * end_pt[0] + p[1] * end_pt[1]) + p[2] * end_pt[2]) - p[3];
            let e = self.extent;
            let push_out = (p[2] * (e[2] * 1.1)).abs() + (p[1] * (e[1] * 1.1)).abs() + (e[0] * 1.1 * p[0]).abs();
            let usable = [d0 <= push_out || d1 <= push_out, -push_out <= d0 || -push_out <= d1];
            let dir = usize::from(d1 <= d0);
            let csg = node.num_vertices != 0 && node.node_flags & 0x21 == 0;
            if usable[dir] {
                let child_outside = if dir == 0 { outside && !csg } else { outside || csg };
                self.box_line_check(i_node, node.child[dir] as i32, child_outside);
            }
            if !usable[1 - dir] {
                return;
            }
            i_parent = i_node;
            i_node = node.child[1 - dir] as i32;
            outside = if dir == 0 { outside || csg } else { outside && !csg };
        }
    }

    /// FBoxLineCheck leaf: contact kept if entry T0 comes before exit T1 and the
    /// best time found so far (Hit.Time).
    fn leaf_line_check(&mut self, i_parent: i32, outside: bool) {
        let m = self.model;
        if outside {
            return;
        }
        let Some(parent) = m.node(i_parent) else { return };
        if parent.collision_bound == -1 {
            return;
        }
        self.setup_hulls(parent);
        self.t0 = -1.0;
        self.t1 = self.hit.time;
        self.hit_normal = [0.0; 3];
        for i in 0..self.hull_planes.len() {
            if !self.clip_to(self.hull_planes[i]) {
                return;
            }
        }
        if !self.bevels_each(|c, p| c.clip_to(p)) {
            return;
        }
        if -1.0 < self.t0 && self.t0 < self.t1 && 0.0 < self.t1 {
            self.hit.time = self.t0;
            self.hit.normal = self.hit_normal;
            self.b_hit = true;
            if self.trace_flags & 0x1000 != 0 {
                // Material: zero-extent trace 200 units down from the start.
                let mut down = BspHit::new(1.0);
                let s = self.start;
                let owner = self.owner.then_some(Frame { to_local: UNIT, to_world: self.coords });
                m.line_check(&mut down, owner.as_ref(), [s[0], s[1], s[2] - 200.0], s, [0.0; 3], self.extra, self.trace_flags);
                if down.time != 1.0 {
                    let surf = m.node(down.item).and_then(|n| usize::try_from(n.surf).ok()).and_then(|i| m.surfs.get(i));
                    self.hit.material = if down.item == -1 { None } else { surf.map(|s| s.material) };
                    self.hit.material_written = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(links: &[([i16; 2], i16)]) -> BspModel {
        let node = |&(child, coplanar): &([i16; 2], i16)| BspNode {
            plane: [0.0; 4],
            node_flags: 0,
            vert_pool: 0,
            surf: 0,
            child,
            coplanar,
            collision_bound: -1,
            zone_mask: 0,
            render_bound: -1,
            sphere: [0.0; 4],
            zone: [0, 1],
            num_vertices: 0,
            leaf: [-1, -1],
        };
        let raw = RawBoundingBox { min: [0.0; 3], max: [0.0; 3], valid: 0 };
        BspModel {
            bounding_box: None,
            raw_bounding_box: raw,
            points: Vec::new(),
            nodes: links.iter().map(node).collect(),
            surfs: Vec::new(),
            verts: Vec::new(),
            zone_actors: Vec::new(),
            zone_connectivity: Vec::new(),
            leaf_hulls: Vec::new(),
            bounds: Vec::new(),
            leaf_zones: Vec::new(),
            root_outside: false,
            polys: 0,
        }
    }

    /// Damaged links are refused at load; a proper tree passes.
    #[test]
    fn check_tree_refuses_bad_links() {
        assert!(tree(&[([1, 2], -1), ([-1, -1], 3), ([-1, -1], -1), ([-1, -1], -1)]).check_tree().is_ok());
        assert!(tree(&[([1, 5], -1), ([-1, -1], -1)]).check_tree().is_err(), "child outside the array");
        assert!(tree(&[([1, -1], -1), ([0, -1], -1)]).check_tree().is_err(), "link to the root");
        assert!(tree(&[([1, 2], -1), ([-1, -1], 2), ([-1, -1], -1)]).check_tree().is_err(), "two links to one node");
        assert!(tree(&[([-1, -1], -1), ([2, -1], -1), ([-1, -1], 1)]).check_tree().is_err(), "loop away from the root");
        let mut m = tree(&[([-1, -1], -1)]);
        m.nodes[0].zone = [0, 64];
        assert!(m.check_tree().is_err(), "zone past 63");
        let mut m = tree(&[([-1, -1], -1)]);
        m.nodes[0].render_bound = 0;
        assert!(m.check_tree().is_err(), "render bound outside the array");
    }

    #[test]
    fn brush_frames_are_inverse() {
        let s = Scale::default();
        let (l, r, pp) = ([32.0, 1184.0, -4320.0], [0, 16384, 0], [-136.0, -8.0, 136.0]);
        let to_local = brush_to_local(l, r, pp, &s, &s);
        let to_world = brush_to_world(l, r, pp, &s, &s);
        let p = [10.0, -20.0, 30.0];
        let w = coords::transform_point(p, &to_world);
        let back = coords::transform_point(w, &to_local);
        for k in 0..3 {
            assert!((back[k] - p[k]).abs() < 1e-3, "{back:?} {p:?}");
        }
        // Without rotation: world = local + Location − PrePivot.
        let w = coords::transform_point(p, &brush_to_world(l, [0; 3], pp, &s, &s));
        assert_eq!(w, [10.0 + 32.0 + 136.0, -20.0 + 1184.0 + 8.0, 30.0 - 4320.0 - 136.0]);
    }

    #[test]
    fn sheer_snap_matches_engine() {
        assert_eq!(sheer_snap(0.0), 0.0);
        assert_eq!(sheer_snap(0.6), 0.5);
        assert_eq!(sheer_snap(-0.7), -0.7 + 0.15);
        assert_eq!(sheer_snap(0.3), 0.3 - 0.05);
    }
}

