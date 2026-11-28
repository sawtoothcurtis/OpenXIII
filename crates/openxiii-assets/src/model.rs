//! A map's BSP geometry (Model class).
//!
//! XIII format (different from UT2003: nodes and surfaces with fixed 16-bit fields):
//! - UPrimitive: FBox (25 bytes), sphere (16), 8 bytes (u32 + f32);
//! - Vectors, Points: compact n × FVector;
//! - Nodes: compact n × 70 bytes: plane (4 × f32), ZoneMask u64, NodeFlags u8,
//!   iVertPool i32, 7 × i16 (iSurf, iBack, iFront, iPlane, ?, ?, ?), sphere (4 × f32),
//!   iZone[2] u8, NumVertices u8, iLeaf[2] i32;
//! - Surfs: compact n × [compact material, PolyFlags u32, 6 × i16 (pBase, vNormal,
//!   vTextureU, vTextureV, iLightMap, iBrushPoly), u8, compact actor, plane (4 × f32)];
//! - Verts: compact n × [pVertex i16, iSide i16].
//!
//! The rest follows UModel::Serialize (Engine.dll) for version 100, licensee 58:
//! NumSharedSides, NumZones, an integer, the zones (actor, two 64-bit masks, float),
//! Polys, LightMap (FLightMapIndex), LightBits (FShadowMask), Bounds,
//! LeafHulls, Leaves, RootOutside, Linked, an integer, the vertex streams (FBspVertexStream:
//! position, colour, U V, U2 V2; 32 bytes copied as is), the lightmap textures
//! (objects) and the static lights (FLight, 0x40 bytes).

use openxiii_package::{Package, Reader, Result};

pub const PF_INVISIBLE: u32 = 0x0000_0001;
pub const PF_NOT_SOLID: u32 = 0x0000_0008;
pub const PF_FAKE_BACKDROP: u32 = 0x0000_0080;
pub const PF_PORTAL: u32 = 0x0400_0000;

#[derive(Debug, Clone)]
pub struct Node {
    pub plane: [f32; 4],
    pub flags: u8,
    pub vert_pool: i32,
    pub surf: i16,
    pub links: [i16; 6],
    pub num_vertices: u8,
    /// iZone[2] (front, back).
    pub zone: [u8; 2],
    /// FBspNode+0x24: sphere bounding the node's polygon (centre, radius).
    pub sphere: [f32; 4],
    /// iLeaf[2].
    pub leaf: [i16; 2],
    /// Vertex stream (FBspNode+0x44) and first vertex in that stream (+0x42); -1 if none.
    pub vertex_stream: i16,
    pub vertex_index: i16,
}

/// BSP vertex ready for rendering (FBspVertex, 32 bytes).
#[derive(Debug, Clone, Copy)]
pub struct BspVertex {
    pub position: [f32; 3],
    /// Colour in memory order (B, G, R, A).
    pub color: [u8; 4],
    pub uv: [f32; 2],
    pub uv2: [f32; 2],
}

/// Zone properties of the model (FZoneProperties).
#[derive(Debug, Clone)]
pub struct ZoneProps {
    pub actor: i32,
    pub connectivity: u64,
    pub visibility: u64,
}

#[derive(Debug, Clone)]
pub struct Surf {
    pub material: i32,
    pub poly_flags: u32,
    pub base: i16,
    pub normal: i16,
    pub texture_u: i16,
    pub texture_v: i16,
    pub light_map: i16,
    pub brush_poly: i16,
    pub actor: i32,
    pub plane: [f32; 4],
    /// FBspSurf+0x34: editor lightmap texture (index into `lightmap_textures`).
    pub lightmap_texture: u8,
}

/// FLightMapIndex (0xC0 bytes): lightmap computed at load time by FLightManager::Cache.
#[derive(Debug, Clone)]
pub struct LightMapIndex {
    /// +0, +4: first entry and number of entries in `light_bits`.
    pub first_light: i32,
    pub num_lights: i32,
    /// +8: WorldToLightMap (FMatrix).
    pub world_to_lightmap: [f32; 16],
    /// +0x88: corner of texel (0, 0); +0x94, +0xA0: step of one texel in U and V.
    pub origin: [f32; 3],
    pub u_step: [f32; 3],
    pub v_step: [f32; 3],
    /// +0xB0, +0xB4: computed texels; +0xB8, +0xBC: texture size.
    pub size: [i32; 4],
}

/// FShadowMask (0x1C bytes): visibility of a light, one bit per texel.
#[derive(Debug, Clone)]
pub struct ShadowMask {
    /// +0: index of the light in `lights`, or -1.
    pub light: i16,
    pub bits: Vec<u8>,
    /// +0x14, +0x18: rows and bytes per row.
    pub rows: i32,
    pub row_bytes: i32,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub vectors: Vec<[f32; 3]>,
    pub points: Vec<[f32; 3]>,
    pub nodes: Vec<Node>,
    pub surfs: Vec<Surf>,
    /// (pVertex, iSide)
    pub verts: Vec<(i16, i16)>,
    pub zones: Vec<ZoneProps>,
    /// Zone of each leaf (FLeaf::iZone).
    pub leaves: Vec<i32>,
    /// FLeaf::iPermeating: start of the leaf's light list in `light_indices`
    /// (terminated by 0xFFFF), -1 without a light.
    pub leaf_permeating: Vec<i32>,
    /// Render vertex streams (FBspVertexStream).
    pub vertex_streams: Vec<Vec<BspVertex>>,
    /// Lightmap textures computed by the editor (UModel+0xEC), referenced by
    /// byte FBspSurf+0x34.
    pub lightmap_textures: Vec<i32>,
    /// UModel+0x118: reference to the model's UPolys (raw compact index).
    pub polys: i32,
    /// UModel+300: RootOutside.
    pub root_outside: bool,
    /// UModel+0x134: lights to refresh from their actors (FLight::Refresh) rather than
    /// from the serialized data (FLight::Init).
    pub refresh_lights: i32,
    /// Static lights (UModel+0x110), of which the first `static_lights` are initialised.
    pub lights: Vec<LightRecord>,
    /// UModel+0x11C.
    pub light_indices: Vec<u16>,
    /// UModel+0xB0: FLightMapIndex; UModel+0xBC: FShadowMask.
    pub light_maps: Vec<LightMapIndex>,
    pub light_bits: Vec<ShadowMask>,
    /// UModel+0x128.
    pub static_lights: i32,
}

/// Serialized data of an FLight (0x40 bytes; colour, radius and animation flags
/// are recomputed by FLight::Init).
#[derive(Debug, Clone)]
pub struct LightRecord {
    pub location: [f32; 3],
    pub direction: [f32; 3],
    pub light_type: u8,
    pub effect: u8,
    pub brightness: u8,
    pub hue: u8,
    pub saturation: u8,
    pub radius: u8,
    pub period: u8,
    pub phase: u8,
    pub cone: u8,
    pub zone: u8,
    /// FLight+0x3C: bDecor1Light… and the actor's flags.
    pub flags: u32,
}

fn vec3(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([f32::from_bits(r.u32()?), f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)])
}

fn vec4(r: &mut Reader) -> Result<[f32; 4]> {
    Ok([f32::from_bits(r.u32()?), f32::from_bits(r.u32()?), f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)])
}

fn i16(r: &mut Reader) -> Result<i16> {
    Ok(r.u16()? as i16)
}

impl Model {
    fn leaf_light_indices(&self, leaf: usize) -> &[u16] {
        let Some(&start) = self.leaf_permeating.get(leaf) else { return &[] };
        let Some(tail) = self.light_indices.get(start as usize..) else { return &[] };
        &tail[..tail.iter().position(|&i| i == u16::MAX).unwrap_or(tail.len())]
    }

    /// Lights (indices into `lights`) that affect leaf `leaf` (list of
    /// LightIndices from FLeaf::iPermeating up to -1).
    pub fn leaf_lights(&self, leaf: usize) -> Vec<usize> {
        self.leaf_light_indices(leaf).iter().map(|&i| i as usize).collect()
    }

    /// The source indices are inserted at the head in the scene context.
    /// An unprocessed leaf has no head; several calls add several copies.
    /// The source contract of leaf_lights remains that of FActorSceneNode::Render.
    pub fn scene_leaf_lights(&self, leaf: usize, treated: &[usize]) -> Vec<usize> {
        if self.leaf_light_indices(leaf).is_empty() { return Vec::new(); }
        self.scene_leaf_lights_visits(leaf, treated.iter().filter(|&&i| i == leaf).count())
    }

    /// `scene_leaf_lights` when the leaf's pass count is already known.
    pub fn scene_leaf_lights_visits(&self, leaf: usize, visits: usize) -> Vec<usize> {
        let source = self.leaf_light_indices(leaf);
        if source.is_empty() { return Vec::new(); }
        let mut out = Vec::with_capacity(source.len() * visits);
        for _ in 0..visits {
            out.extend(source.iter().rev().map(|&i| i as usize));
        }
        out
    }

    pub fn read(package: &Package, data: &[u8], index: usize) -> Result<Self> {
        Ok(Self::read_pos(package, data, index)?.0)
    }

    /// `read`, with the position where reading ends (the serializer test checks that it
    /// lands on the end of the object, like UModel::Serialize).
    pub fn read_pos(package: &Package, data: &[u8], index: usize) -> Result<(Self, usize)> {
        let object = package.object_data(data, index)?;
        let export = &package.exports[index];
        let end = export.serial_offset as usize + export.serial_size as usize;
        let mut r = Reader::new(&data[..end], object.end);
        r.take(25 + 16 + 8)?;
        let vectors = (0..r.compact()?).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;
        let points = (0..r.compact()?).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;
        let nodes = (0..r.compact()?)
            .map(|_| {
                let plane = vec4(&mut r)?;
                r.take(8)?; // ZoneMask
                let flags = r.u8()?;
                let vert_pool = r.i32()?;
                let surf = i16(&mut r)?;
                let mut links = [0i16; 6];
                for l in &mut links {
                    *l = i16(&mut r)?;
                }
                let sphere = vec4(&mut r)?;
                let zone = [r.u8()?, r.u8()?];
                let num_vertices = r.u8()?;
                let leaf = [i16(&mut r)?, i16(&mut r)?];
                let vertex_stream = i16(&mut r)?;
                let vertex_index = i16(&mut r)?;
                Ok(Node { plane, flags, vert_pool, surf, links, num_vertices, zone, sphere, leaf, vertex_stream, vertex_index })
            })
            .collect::<Result<Vec<_>>>()?;
        let surfs = (0..r.compact()?)
            .map(|_| {
                let material = r.compact()?;
                let poly_flags = r.u32()?;
                let base = i16(&mut r)?;
                let normal = i16(&mut r)?;
                let texture_u = i16(&mut r)?;
                let texture_v = i16(&mut r)?;
                let light_map = i16(&mut r)?;
                let brush_poly = i16(&mut r)?;
                let lightmap_texture = r.u8()?;
                let actor = r.compact()?;
                let plane = vec4(&mut r)?;
                Ok(Surf { material, poly_flags, base, normal, texture_u, texture_v, light_map, brush_poly, actor, plane, lightmap_texture })
            })
            .collect::<Result<Vec<_>>>()?;
        let verts = (0..r.compact()?).map(|_| Ok((i16(&mut r)?, i16(&mut r)?))).collect::<Result<Vec<_>>>()?;
        // NumSharedSides, NumZones, licensee integer (> 0x2C).
        r.i32()?;
        let num_zones = r.i32()?;
        r.i32()?;
        let zones = (0..num_zones.max(0))
            .map(|_| {
                let actor = r.compact()?;
                let connectivity = r.u32()? as u64 | (r.u32()? as u64) << 32;
                let visibility = r.u32()? as u64 | (r.u32()? as u64) << 32;
                r.u32()?;
                Ok(ZoneProps { actor, connectivity, visibility })
            })
            .collect::<Result<Vec<_>>>()?;
        let polys = r.compact()?; // Polys
                      // LightMap: FLightMapIndex: two integers, two FMatrix, two bytes,
                      // 9 floats (origin, U step, V step), 4 indices.
        let version = package.header.version;
        let light_maps = if version < 92 {
            // These old indices are read into a temporary
            // array then destroyed, not into the model's LightMaps.
            for _ in 0..r.compact()? {
                r.take(16)?; // Four old integers.
                r.compact()?;
                r.compact()?;
                r.take(12)?; // Three old integers.
            }
            Vec::new()
        } else {
            (0..r.compact()?)
                .map(|_| {
                    let first_light = r.i32()?;
                    let num_lights = r.i32()?;
                    let mut m = [0.0f32; 16];
                    for v in m.iter_mut() {
                        *v = f32::from_bits(r.u32()?);
                    }
                    let mut inverse = [0.0f32; 16];
                    for v in inverse.iter_mut() {
                        *v = f32::from_bits(r.u32()?);
                    }
                    r.take(2)?;
                    let (origin, u_step, v_step) = if version >= 96 {
                        (vec3(&mut r)?, vec3(&mut r)?, vec3(&mut r)?)
                    } else {
                        // The old format derives these vectors
                        // from the second matrix. Same x87 order: (z+y)+x.
                        let transform = |v: [f32; 3], translate: bool| {
                            std::array::from_fn(|j| {
                                let z = inverse[8 + j] * v[2];
                                let y = inverse[4 + j] * v[1];
                                let zy = z + y;
                                let x = inverse[j] * v[0];
                                let xyz = zy + x;
                                if translate { xyz + inverse[12 + j] } else { xyz }
                            })
                        };
                        (transform([0.0, 0.0, 0.0], true),
                         transform([1.0, 0.0, 0.0], false),
                         transform([0.0, 1.0, 0.0], false))
                    };
                    let size = [r.compact()?, r.compact()?, r.compact()?, r.compact()?];
                    Ok(LightMapIndex { first_light, num_lights, world_to_lightmap: m, origin, u_step, v_step, size })
                })
                .collect::<Result<Vec<_>>>()?
        };
        // LightBits: FShadowMask: light (u16), bits, three integers.
        let light_bits = if version < 92 {
            // The second temporary array is a TArray<BYTE>.
            let count = r.compact()?.max(0) as usize;
            r.take(count)?;
            Vec::new()
        } else {
            (0..r.compact()?)
                .map(|_| {
                    let light = r.u16()? as i16;
                    let n = r.compact()?;
                    let bits = r.take(n.max(0) as usize)?.to_vec();
                    let _ = r.i32()?;
                    let rows = r.i32()?;
                    let row_bytes = r.i32()?;
                    Ok(ShadowMask { light, bits, rows, row_bytes })
                })
                .collect::<Result<Vec<_>>>()?
        };
        let n = r.compact()?; // Bounds (FBox)
        r.take(n.max(0) as usize * 25)?;
        let n = r.compact()?; // LeafHulls
        r.take(n.max(0) as usize * 4)?;
        let leaf_data = (0..r.compact()?)
            .map(|_| {
                // Leaves: iZone, iPermeating, iVolumetric (compact indices), VisibleZones (64 bits).
                let zone = r.compact()?;
                let permeating = r.compact()?;
                r.compact()?;
                r.take(8)?;
                Ok((zone, permeating))
            })
            .collect::<Result<Vec<_>>>()?;
        let leaves = leaf_data.iter().map(|&(z, _)| z).collect();
        let leaf_permeating = leaf_data.iter().map(|&(_, p)| p).collect();
        let root_outside = r.i32()? != 0;
        r.i32()?; // Linked
        let refresh_lights = r.i32()?;
        if version == 92 {
            // a temporary stream, then its revision integer.
            let count = r.compact()?.max(0) as usize;
            r.take(count * 32)?;
            r.i32()?;
        }
        let vertex_streams = if version >= 93 {
            (0..r.compact()?)
                .map(|_| {
                    let n = r.compact()?.max(0) as usize;
                    // TArray<FBspVertex>: LicenseeVer < 34, one vertex after another.
                    if package.header.licensee < 34 {
                        let v = (0..n).map(|_| bsp_vertex_old(&mut r, package.header.licensee)).collect::<Result<Vec<_>>>()?;
                        r.i32()?;
                        return Ok(v);
                    }
                    let raw = r.take(n * 32)?;
                    r.i32()?;
                    Ok(raw
                        .as_chunks::<32>()
                        .0
                        .iter()
                        .map(|c| {
                            let f = |o: usize| f32::from_le_bytes([c[o], c[o + 1], c[o + 2], c[o + 3]]);
                            BspVertex { position: [f(0), f(4), f(8)], color: [c[12], c[13], c[14], c[15]], uv: [f(16), f(20)], uv2: [f(24), f(28)] }
                        })
                        .collect())
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let lightmap_textures = (0..r.compact()?).map(|_| r.compact()).collect::<Result<Vec<_>>>()?;
        let lights = (0..r.compact()?)
            .map(|_| {
                let location = vec3(&mut r)?;
                let direction = vec3(&mut r)?;
                let b = r.take(9)?.to_vec();
                let flags = r.u32()?;
                let zone = r.u8()?;
                Ok(LightRecord {
                    location,
                    direction,
                    light_type: b[0],
                    effect: b[1],
                    brightness: b[2],
                    hue: b[3],
                    saturation: b[4],
                    radius: b[5],
                    period: b[6],
                    phase: b[7],
                    cone: b[8],
                    zone,
                    flags,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let light_indices = (0..r.compact()?).map(|_| r.u16()).collect::<Result<Vec<_>>>()?;
        let static_lights = r.i32()?;
        let model = Model {
            vectors,
            points,
            nodes,
            surfs,
            verts,
            zones,
            leaves,
            leaf_permeating,
            vertex_streams,
            lightmap_textures,
            polys,
            root_outside,
            refresh_lights,
            lights,
            light_indices,
            light_maps,
            light_bits,
            static_lights,
        };
        model.validate()?;
        Ok((model, r.pos))
    }

    /// The object references UModel::Serialize sees in file order: Material and Actor of each
    /// surface, Actor of each zone, Polys, then the lightmap textures. This is what
    /// `operator<<(UObject*&)` receives when the garbage collector walks the model (raw compact indices).
    pub fn gc_refs(&self) -> Vec<i32> {
        let mut r: Vec<i32> = self.surfs.iter().flat_map(|s| [s.material, s.actor]).collect();
        r.extend(self.zones.iter().map(|z| z.actor));
        r.push(self.polys);
        r.extend(self.lightmap_textures.iter().copied());
        r
    }

    fn validate(&self) -> Result<()> {
        let bad = |v: i64| openxiii_package::Error::BadIndex(v as i32);
        // Links walked by the lighting and lightmap passes (iBack, iFront, iPlane).
        crate::bsp::check_node_links(self.nodes.len(), self.nodes.iter().map(|n| [n.links[0], n.links[1], n.links[2]])).map_err(bad)?;
        for n in &self.nodes {
            if n.num_vertices > 0 {
                if n.surf < 0 || n.surf as usize >= self.surfs.len() {
                    return Err(bad(n.surf as i64));
                }
                let end = n.vert_pool as i64 + n.num_vertices as i64;
                if n.vert_pool < 0 || end > self.verts.len() as i64 {
                    return Err(bad(end));
                }
                // Only the vertices used by a node have to be valid: XIII's Verts
                // array also contains orphan entries outside Points.
                let used = &self.verts[n.vert_pool as usize..end as usize];
                if let Some(&(p, _)) = used.iter().find(|(p, _)| *p < 0 || *p as usize >= self.points.len()) {
                    return Err(bad(p as i64));
                }
            }
        }
        for s in &self.surfs {
            if s.base < 0 || s.base as usize >= self.points.len() {
                return Err(bad(s.base as i64));
            }
            for v in [s.normal, s.texture_u, s.texture_v] {
                if v < 0 || v as usize >= self.vectors.len() {
                    return Err(bad(v as i64));
                }
            }
        }
        Ok(())
    }

    /// Visible polygons (without invisible surfaces, portals or fake backdrop):
    /// (surface index, vertices in node order).
    pub fn polygons(&self) -> Vec<(usize, Vec<[f32; 3]>)> {
        self.nodes
            .iter()
            .filter(|n| n.num_vertices >= 3)
            .filter_map(|n| {
                let surf = n.surf as usize;
                let flags = self.surfs[surf].poly_flags;
                if flags & (PF_INVISIBLE | PF_PORTAL | PF_FAKE_BACKDROP) != 0 {
                    return None;
                }
                let start = n.vert_pool as usize;
                let poly = self.verts[start..start + n.num_vertices as usize].iter().map(|&(p, _)| self.points[p as usize]).collect();
                Some((surf, poly))
            })
            .collect()
    }

    /// Blocking polygons (invisible ones included; without portals, fake backdrop or NotSolid).
    /// Like `collision_polygons`, with the surface index (for the material).
    pub fn collision_polygons_with_surfs(&self) -> Vec<(usize, Vec<[f32; 3]>)> {
        self.nodes
            .iter()
            .filter(|n| n.num_vertices >= 3)
            .filter(|n| self.surfs[n.surf as usize].poly_flags & (PF_NOT_SOLID | PF_PORTAL | PF_FAKE_BACKDROP) == 0)
            .map(|n| {
                let start = n.vert_pool as usize;
                (n.surf as usize, self.verts[start..start + n.num_vertices as usize].iter().map(|&(p, _)| self.points[p as usize]).collect())
            })
            .collect()
    }

    pub fn collision_polygons(&self) -> Vec<Vec<[f32; 3]>> {
        self.nodes
            .iter()
            .filter(|n| n.num_vertices >= 3)
            .filter(|n| self.surfs[n.surf as usize].poly_flags & (PF_NOT_SOLID | PF_PORTAL | PF_FAKE_BACKDROP) == 0)
            .map(|n| {
                let start = n.vert_pool as usize;
                self.verts[start..start + n.num_vertices as usize].iter().map(|&(p, _)| self.points[p as usize]).collect()
            })
            .collect()
    }

    /// Unreal texture coordinates (in texels) of a point on a surface.
    pub fn texel(&self, surf: &Surf, p: [f32; 3]) -> [f32; 2] {
        let base = self.points[surf.base as usize];
        let (u, v) = (self.vectors[surf.texture_u as usize], self.vectors[surf.texture_v as usize]);
        let d = [p[0] - base[0], p[1] - base[1], p[2] - base[2]];
        let dot = |a: [f32; 3]| d[0] * a[0] + d[1] * a[1] + d[2] * a[2];
        [dot(u), dot(v)]
    }
}

/// The map's BSP model: the largest Model that is not any actor's `Brush` property.
pub fn level_model(map: &Package, data: &[u8]) -> Option<usize> {
    use openxiii_package::{ObjectRef, Value};
    let mut brushes = std::collections::HashSet::new();
    for i in 0..map.exports.len() {
        if map.export_class(i) == "Class" || map.exports[i].serial_size == 0 {
            continue;
        }
        if let Ok(o) = map.object_data(data, i) {
            if let Some(Value::Object(b)) = o.get(map, "Brush") {
                if let ObjectRef::Export(e) = ObjectRef::from(*b) {
                    brushes.insert(e);
                }
            }
        }
    }
    (0..map.exports.len()).filter(|&i| map.export_class(i) == "Model" && !brushes.contains(&i)).max_by_key(|&i| map.exports[i].serial_size)
}

/// Old FBspVertex, read into a zeroed vertex (AddZeroed of the loader): position
/// (3 floats); LicenseeVer >= 34: colour in memory +14, +13, +12, +15; UV and UV2 (4 floats); then,
/// below 34, the colour after the UVs (same order) from 25 on, otherwise FFFFFFFF on load.
pub(crate) fn bsp_vertex_old(r: &mut Reader, licensee: u16) -> Result<BspVertex> {
    let mut f = || -> Result<f32> { let b = r.take(4)?; Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]])) };
    let position = [f()?, f()?, f()?];
    let mut color = [0u8; 4];
    let mut color_value = |r: &mut Reader, c: &mut [u8; 4]| -> Result<()> {
        let b = r.take(4)?;
        (c[2], c[1], c[0], c[3]) = (b[0], b[1], b[2], b[3]);
        Ok(())
    };
    if licensee >= 34 {
        color_value(r, &mut color)?;
    }
    let mut f = || -> Result<f32> { let b = r.take(4)?; Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]])) };
    let (u, v, u2, v2) = (f()?, f()?, f()?, f()?);
    if licensee < 34 {
        if licensee >= 25 {
            color_value(r, &mut color)?;
        } else {
            color = [0xFF; 4];
        }
    }
    Ok(BspVertex { position, color, uv: [u, v], uv2: [u2, v2] })
}

