//! Engine.dll terrain (ATerrainInfo, UTerrainSector): serialized data and render geometry.
//!
//! ATerrainInfo::Serialize: after the properties, Sectors, Vertices (positions
//! computed by CalcVertices), SectorsX/Y, ToWorld, ToHeightmap, HeightmapX/Y. Each
//! UTerrainSector (Serialize) gives its quad rectangle, its lights and the colours
//! of its vertices computed by the editor (UTerrainSector::StaticLight, replayed at
//! render time only for 0x800 lights). UTerrainSector::UpdateVertexBuffer builds
//! the vertices and, for each layer, the triangles of the quads where it is visible; ATerrainInfo::Render
//! draws each layer with the material from UTerrainRenderMaterial::SetTerrain

use crate::coords::{self, Coords};
use crate::texture::Texture;
use openxiii_package::library::Library;
use openxiii_package::{ObjectRef, Package, Reader, Value};

/// Terrain layer (TerrainLayer, 0x2C bytes at ATerrainInfo+0x200).
#[derive(Debug, Clone, Default)]
pub struct Layer {
    pub texture: Option<String>,
    pub alpha_map: Option<String>,
    pub uscale: f32,
    pub vscale: f32,
    pub upan: f32,
    pub vpan: f32,
    pub map_axis: u8,
    pub rotation: f32,
    pub layer_rotation: [i32; 3],
}

/// A layer's alpha map as GetLayerAlpha reads it: USize, VSize, format
/// (0: byte, 5: RGBA8 using its alpha) and raw data of the first mip.
#[derive(Debug, Clone)]
pub struct AlphaMap {
    pub usize: i32,
    pub vsize: i32,
    pub format: u8,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SectorLight {
    pub index: u16,
    pub visibility: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Sector {
    /// Package reference of the owning TerrainInfo (+0x70).
    pub terrain_reference: i32,
    pub quads_x: i32,
    pub quads_y: i32,
    pub offset_x: i32,
    pub offset_y: i32,
    /// +0x74: eight corners of the sector box (FBox of eight points).
    pub bounds: [[f32; 3]; 8],
    /// FRawColorStream (+0x40): vertex colours (B, G, R, A).
    pub colors: Vec<[u8; 4]>,
    /// +0x64: light indices and visibility kept when reading.
    pub lights: Vec<SectorLight>,
    /// Revision of the FRawColorStream (+0x54), copied from COLOR before version 89.
    pub color_revision: u32,
}

/// Native data of an ATerrainInfo.
#[derive(Debug, Clone)]
pub struct TerrainData {
    pub heightmap_x: i32,
    pub heightmap_y: i32,
    pub vertices: Vec<[f32; 3]>,
    pub to_world: Coords,
    pub to_heightmap: Coords,
    pub sectors_x: i32,
    pub sectors_y: i32,
    pub sectors: Vec<Sector>,
    /// End of the native part (LicenseeVer >= 12): references (package
    /// object indices) of FootstepSound (+0x850), XIIIFootStepSound (+0x854), HitSound
    /// (+0x860), PNJSndStep (+0x858) and XIIISndStep (+0x85C), and NoiseLoudness (+0x864). Read
    /// after the properties, they override them.
    pub sounds: [i32; 5],
    pub noise_loudness: f32,
}

/// ATerrainInfo::GetGlobalVertex: index of vertex (x, y) in the height map, HeightmapX × y + x.
/// ATerrainInfo::PostLoad (bitmaps): QuadVisibilityBitmap and EdgeTurnBitmap of
/// (HeightmapX × HeightmapY) / 32 words, rebuilt (fully visible, no turn) when their size differs.
pub fn post_load_bitmaps(visible: Vec<u32>, turned: Vec<u32>, heightmap_x: i32, heightmap_y: i32) -> (Vec<u32>, Vec<u32>) {
    let n = heightmap_x.wrapping_mul(heightmap_y);
    let words = ((n + if n < 0 { 31 } else { 0 }) >> 5).max(0) as usize;
    let visible = if visible.len() == words { visible } else { vec![0xFFFF_FFFF; words] };
    let turned = if turned.len() == words { turned } else { vec![0; words] };
    (visible, turned)
}

/// Height map point (HeightmapX / 2, HeightmapY / 2, 32767) passed through the ATerrainInfo's coordinates at +0x7B0 (
/// (p − origin) projected on the three axes: component k = dot product with axis k, (d0 × A0 + d1 × A1) + d2 × A2), 24-bit FPU. `coords`: origin, X axis, Y axis, Z axis (12 floats).
pub fn terrain_center(heightmap_x: i32, heightmap_y: i32, coords: &[f32; 12]) -> [f32; 3] {
    let p = [(heightmap_x / 2) as f32, (heightmap_y / 2) as f32, 32767.0];
    let d = [p[0] - coords[0], p[1] - coords[1], p[2] - coords[2]];
    let axe = |k: usize| (d[0] * coords[3 + 3 * k] + d[1] * coords[4 + 3 * k]) + d[2] * coords[5 + 3 * k];
    [axe(0), axe(1), axe(2)]
}

/// ATerrainInfo::PostLoad: "Terrain origin is incorrect" when Location is further than
/// sqrt(5) from the centre of the height map (squared distance > 5); Location is then reset to that centre.
/// None if the origin is correct.
pub fn post_load_origin(heightmap_x: i32, heightmap_y: i32, coords: &[f32; 12], location: [f32; 3]) -> Option<[f32; 3]> {
    let c = terrain_center(heightmap_x, heightmap_y, coords);
    let (dx, dy, dz) = (location[0] - c[0], location[1] - c[1], location[2] - c[2]);
    let d2 = (dx * dx + dy * dy) + dz * dz;
    (d2 > 5.0).then_some(c)
}

/// ATerrainInfo::PostLoad: the two GLog->Logf lines (NAME_Log, 0x2F8, hence "Log: "), the second
/// using msvcr70's "%f" on the three centre floats passed as doubles.
pub fn post_load_origin_log(c: [f32; 3]) -> [String; 2] {
    use crate::printf::printf_f;
    ["Log: Terrain origin is incorrect".into(), format!("Log: Resetting terrain origin to ({}, {}, {})", printf_f(c[0]), printf_f(c[1]), printf_f(c[2]))]
}

/// ATerrainInfo::PostLoad in full, except the height map itself: sequence of calls, recorded as
/// W and V(x, y, z) (warnings, doubles), T (height map texture created when
/// +0x86C is not empty), A, M, N (rebuild when HeightmapX ≠ 8 × NumSectorsX or HeightmapY ≠ 8 × NumSectorsY), Q
/// (ComputeQuadMaterials, always last).
pub fn post_load_events(heightmap_x: i32, heightmap_y: i32, sectors_x: i32, sectors_y: i32, heightmap_bytes: usize, coords: &[f32; 12], location: [f32; 3]) -> (Vec<String>, [f32; 3]) {
    let mut ev = Vec::new();
    let mut loc = location;
    if let Some(c) = post_load_origin(heightmap_x, heightmap_y, coords, location) {
        ev.push("W".to_string());
        ev.push(format!("V({:016x},{:016x},{:016x})", (c[0] as f64).to_bits(), (c[1] as f64).to_bits(), (c[2] as f64).to_bits()));
        loc = c;
    }
    if heightmap_bytes > 0 {
        ev.push("T".into());
    }
    if heightmap_x != sectors_x.wrapping_mul(8) || heightmap_y != sectors_y.wrapping_mul(8) {
        ev.extend(["A", "M", "N"].map(String::from));
    }
    ev.push("Q".into());
    (ev, loc)
}

/// Size of the quad material table (+0x838) after PostLoad: (HeightmapX × HeightmapY) / 2 bytes.
pub fn post_load_quad_material_len(heightmap_x: i32, heightmap_y: i32) -> i32 {
    let n = heightmap_x.wrapping_mul(heightmap_y);
    (n + if n < 0 { 1 } else { 0 }) >> 1
}

pub fn global_vertex(heightmap_x: i32, x: i32, y: i32) -> i32 {
    heightmap_x.wrapping_mul(y).wrapping_add(x)
}

/// UTerrainSector::GetGlobalVertex: index in the height map of the sector's vertex (x, y),
/// (OffsetY + y) × HeightmapX + OffsetX + x.
pub fn sector_global_vertex(heightmap_x: i32, offset_x: i32, offset_y: i32, x: i32, y: i32) -> i32 {
    offset_y.wrapping_add(y).wrapping_mul(heightmap_x).wrapping_add(offset_x).wrapping_add(x)
}

fn f32r(r: &mut Reader) -> openxiii_package::Result<f32> {
    Ok(f32::from_bits(r.u32()?))
}

fn coords_r(r: &mut Reader) -> openxiii_package::Result<Coords> {
    let mut v = [0.0f32; 12];
    for x in &mut v {
        *x = f32r(r)?;
    }
    Ok(Coords { origin: [v[0], v[1], v[2]], x: [v[3], v[4], v[5]], y: [v[6], v[7], v[8]], z: [v[9], v[10], v[11]] })
}

/// UTerrainSector::Serialize: TerrainInfo, QuadsX, QuadsY,
/// OffsetX, OffsetY, box (8 FVector), lights (u16 index, TArray<BYTE> visibility),
/// colours (FRawColorStream).
pub fn read_sector(package: &Package, data: &[u8], index: usize) -> openxiii_package::Result<Sector> {
    read_sector_with_position(package, data, index).map(|(sector, _)| sector)
}

/// Same production loading, with its position in the native part.
pub fn read_sector_with_position(package: &Package, data: &[u8], index: usize) -> openxiii_package::Result<(Sector, usize)> {
    let object = package.object_data(data, index)?;
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let version = package.header.version;
    read_sector_version(&data[object.end..end], version, package.header.licensee, |reference| {
        let ObjectRef::Export(i) = ObjectRef::from(reference) else {
            return Err(openxiii_package::Error::BadIndex(reference));
        };
        let source = package.object_data(data, i)?;
        let source_export = &package.exports[i];
        let source_end = source_export.serial_offset as usize + source_export.serial_size as usize;
        let mut r = Reader::new(&data[..source_end], source.end);
        // UVertexStreamCOLOR: base revision (+0x28), then three stream words
        // from version 75; FColor is archived R,G,B,A and stored B,G,R,A.
        let revision = r.u32()?;
        if version >= 75 { r.take(12)?; }
        let n = r.compact()?.max(0) as usize;
        let mut colors = Vec::with_capacity(n);
        for _ in 0..n {
            let c = r.take(4)?;
            colors.push([c[2], c[1], c[0], c[3]]);
        }
        Ok((colors, revision))
    })
}

/// Native part of a modern UTerrainSector (version ≥ 94, after the properties).
/// Also returns the bytes read; old objects go through `read_sector`.
pub fn read_sector_native(native: &[u8]) -> openxiii_package::Result<(Sector, usize)> {
    read_sector_version(native, 94, 58, |i| Err(openxiii_package::Error::BadIndex(i)))
}

fn read_sector_version(native: &[u8], version: u16, licensee: u16, legacy_colors: impl FnOnce(i32) -> openxiii_package::Result<(Vec<[u8; 4]>, u32)>) -> openxiii_package::Result<(Sector, usize)> {
    let mut r = Reader::new(native, 0);
    // FBox (25 bytes) and FSphere (12 up to 61, then 16).
    if version < 94 { r.take(25 + if version <= 61 { 12 } else { 16 })?; }
    let terrain_reference = r.compact()?;
    let (quads_x, quads_y, offset_x, offset_y) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
    let mut bounds = [[0.0f32; 3]; 8];
    for p in &mut bounds {
        *p = [f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?];
    }
    let mut lights = Vec::new();
    for _ in 0..r.compact()? {
        // Before licensee 50, the reference is consumed then
        // replaced by INDEX_NONE; from 50, only the WORD index is archived.
        let index = if licensee < 50 { r.compact()?; u16::MAX } else { r.u16()? };
        let n = r.compact()?.max(0) as usize;
        let visibility = r.take(n)?.to_vec();
        lights.push(SectorLight { index, visibility });
    }
    let (colors, color_revision) = if version < 89 {
        // Reference to the old stream, then copy of its colours.
        legacy_colors(r.compact()?)?
    } else {
        let n = r.compact()?.max(0) as usize;
        // a persistent read sets the revision to 1.
        (r.take(n * 4)?.as_chunks::<4>().0.to_vec(), 1)
    };
    Ok((Sector { terrain_reference, quads_x, quads_y, offset_x, offset_y, bounds, colors, lights, color_revision }, r.pos))
}

/// ATerrainInfo::Serialize (version 100): native data after the properties.
pub fn read_terrain(map: &Package, data: &[u8], index: usize) -> openxiii_package::Result<TerrainData> {
    read_terrain_with_visibility(map, data, index).map(|(terrain, _)| terrain)
}

/// The native visibility of versions 86/87 replaces the earlier property.
pub fn read_terrain_with_visibility(map: &Package, data: &[u8], index: usize) -> openxiii_package::Result<(TerrainData, Option<Vec<u32>>)> {
    let object = map.object_data(data, index)?;
    let export = &map.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let (sector_refs, mut t, _, visibility) = read_terrain_native_version_with_visibility(&data[object.end..end], map.header.version, map.header.licensee)?;
    for s in sector_refs {
        if let ObjectRef::Export(i) = ObjectRef::from(s) {
            t.sectors.push(read_sector(map, data, i)?);
        }
    }
    Ok((t, visibility))
}

/// Native part of an ATerrainInfo (after the properties), without the sectors: sector
/// references, data, bytes read.
pub fn read_terrain_native(native: &[u8]) -> openxiii_package::Result<(Vec<i32>, TerrainData, usize)> {
    read_terrain_native_version(native, 100, 58)
}

/// Persistent read: the old transient arrays are read then destroyed by
/// ATerrainInfo::Serialize. The sound references depend on the licensee version.
pub fn read_terrain_native_version(native: &[u8], version: u16, licensee: u16) -> openxiii_package::Result<(Vec<i32>, TerrainData, usize)> {
    read_terrain_native_version_with_visibility(native, version, licensee).map(|(refs, terrain, end, _)| (refs, terrain, end))
}

/// None keeps the property, Some (even empty) represents the native array.
pub fn read_terrain_native_version_with_visibility(native: &[u8], version: u16, licensee: u16) -> openxiii_package::Result<(Vec<i32>, TerrainData, usize, Option<Vec<u32>>)> {
    read_terrain_native_for_postload(native, version, licensee).map(|(refs, terrain, end, visibility, _)| (refs, terrain, end, visibility))
}

/// Retains the legacy height words until Terrain PostLoad has copied them to G16.
pub fn read_terrain_native_for_postload(native: &[u8], version: u16, licensee: u16) -> openxiii_package::Result<(Vec<i32>, TerrainData, usize, Option<Vec<u32>>, Vec<u8>)> {
    let mut r = Reader::new(native, 0);
    let sector_refs: Vec<i32> = (0..r.compact()?).map(|_| r.compact()).collect::<openxiii_package::Result<_>>()?;
    let vertices: Vec<[f32; 3]> = (0..r.compact()?).map(|_| Ok([f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?])).collect::<openxiii_package::Result<_>>()?;
    let (sectors_x, sectors_y) = (r.i32()?, r.i32()?);
    if licensee < 20 {
        let count = r.compact()?.max(0) as usize;
        r.take(count * 24)?; // Six words per element.
    }
    if version < 83 {
        let count = r.compact()?.max(0) as usize;
        r.take(count * 16)?; // Four words per element.
    }
    let to_world = coords_r(&mut r)?;
    let to_heightmap = coords_r(&mut r)?;
    let old_heightmap = if version < 76 {
        let count = r.compact()?.max(0) as usize;
        r.take(count * 2)?.to_vec() // Array of 16-bit words.
    } else { Vec::new() };
    let (heightmap_x, heightmap_y) = (r.i32()?, r.i32()?);
    if (75..82).contains(&version) {
        for _ in 0..16 {
            r.take(28)?;
            r.string()?;
        }
    }
    let visibility = if version == 86 || version == 87 {
        let count = r.compact()?.max(0) as usize;
        Some((0..count).map(|_| r.u32()).collect::<openxiii_package::Result<Vec<_>>>()?)
    } else {
        None
    };
    let (mut sounds, mut noise_loudness) = ([0i32; 5], 0.0);
    if licensee >= 12 && r.pos < native.len() {
        sounds[0] = r.compact()?;
        sounds[1] = r.compact()?;
        if licensee < 28 {
            r.compact()?;
            r.compact()?;
        }
        if licensee < 17 {
            r.compact()?;
            r.compact()?;
        }
        sounds[2] = r.compact()?;
        noise_loudness = f32r(&mut r)?;
        if licensee >= 36 {
            sounds[3] = r.compact()?;
            sounds[4] = r.compact()?;
        }
    }
    let t = TerrainData { heightmap_x, heightmap_y, vertices, to_world, to_heightmap, sectors_x, sectors_y, sectors: Vec::new(), sounds, noise_loudness };
    Ok((sector_refs, t, r.pos, visibility, old_heightmap))
}

/// Layers `Layers[i]` of a TerrainInfo (native format: texture, alpha map (compact), UScale,
/// VScale, UPan, VPan (f32), TextureMapAxis (u8), TextureRotation (f32), LayerRotation (3 × i32)).
pub fn read_layers(map_name: &str, map: &Package, o: &openxiii_package::ObjectData) -> Vec<Layer> {
    let mut layers = vec![Layer::default(); 32];
    for p in o.properties.iter().filter(|p| map.name(p.name).eq_ignore_ascii_case("Layers")) {
        let Value::Raw(raw) = &p.value else { continue };
        let mut r = Reader::new(raw, 0);
        let mut read = || -> openxiii_package::Result<Layer> {
            let texture = r.compact()?;
            let alpha = r.compact()?;
            let path = |x: i32| (x != 0).then(|| crate::world::qualified(map_name, map, x));
            Ok(Layer {
                texture: path(texture),
                alpha_map: path(alpha),
                uscale: f32r(&mut r)?,
                vscale: f32r(&mut r)?,
                upan: f32r(&mut r)?,
                vpan: f32r(&mut r)?,
                map_axis: r.u8()?,
                rotation: f32r(&mut r)?,
                layer_rotation: [r.i32()?, r.i32()?, r.i32()?],
            })
        };
        if let (Ok(l), Some(slot)) = (read(), layers.get_mut(p.array_index as usize)) {
            *slot = l;
        }
    }
    layers
}

/// Alpha map (texture) read for GetLayerAlpha.
pub fn load_alpha_map(library: &mut Library, path: &str) -> Option<AlphaMap> {
    let (loaded, index) = library.resolve(path)?;
    let o = loaded.package.object_data(&loaded.data, index).ok()?;
    let int = |n: &str| match o.get(&loaded.package, n) {
        Some(Value::Int(v)) => Some(*v),
        _ => None,
    };
    let t = Texture::read(&loaded.package, &loaded.data, index).ok()?;
    let mip = t.mips.first()?;
    let format = match t.format {
        crate::texture::Format::P8 => 0,
        crate::texture::Format::Rgba8 => 5,
        _ => 0xFF,
    };
    Some(AlphaMap { usize: int("USize").unwrap_or(mip.width as i32), vsize: int("VSize").unwrap_or(mip.height as i32), format, data: mip.data.clone() })
}

impl TerrainData {
    /// ATerrainInfo::GetLayerAlpha with a given map.
    pub fn layer_alpha(&self, x: i32, y: i32, map: &AlphaMap) -> u8 {
        let ix = (map.usize * x) / self.heightmap_x.max(1);
        let iy = (map.vsize * y) / self.heightmap_y.max(1);
        match map.format {
            0 => map.data.get((ix + map.usize * iy) as usize).copied().unwrap_or(0),
            5 => map.data.get(((map.usize * iy + ix) * 4 + 3) as usize).copied().unwrap_or(0),
            _ => 0,
        }
    }

    /// ATerrainInfo::ComputeQuadMaterials: material of each quad (layer index,
    /// 0 without a layer). `maps`: alpha maps of the layers that have a texture and a map (the first 16
    /// count). Average >> 2 of the alphas of the quad's corners (edges: existing corners
    /// only); a layer takes the quad if its average equals or exceeds the best one.
    pub fn quad_materials(&self, maps: &[Option<AlphaMap>]) -> Vec<u8> {
        let (hx, hy) = (self.heightmap_x, self.heightmap_y);
        let quads = (hx.max(0) * hy.max(0)) as usize;
        let mut best = vec![0u8; quads];
        let mut material = vec![0u8; quads];
        for (l, map) in maps.iter().take(16).enumerate() {
            let Some(map) = map else { continue };
            for y in 0..hy {
                for x in 0..hx {
                    let al = |x: i32, y: i32| self.layer_alpha(x, y, map) as u32;
                    let mut sum = al(x, y);
                    if x < hx - 1 {
                        sum += al(x + 1, y);
                        if y < hy - 1 {
                            sum += al(x, y + 1) + al(x + 1, y + 1);
                        }
                    } else if y < hy - 1 {
                        sum += al(x, y + 1);
                    }
                    let q = (hx * y + x) as usize;
                    if best[q] as u32 <= sum >> 2 {
                        best[q] = (sum >> 2) as u8;
                        material[q] = l as u8;
                    }
                }
            }
        }
        material
    }

    fn bit(bitmap: &[u32], i: i32) -> bool {
        bitmap.get((i >> 5) as usize).is_some_and(|w| w & (1 << (i & 31)) != 0)
    }
}

/// Layer to draw in a sector: layer index and triangles (indices local to the sector).
#[derive(Debug, Clone)]
pub struct SectorLayer {
    pub layer: usize,
    pub indices: Vec<u32>,
}

/// UTerrainSector::UpdateVertexBuffer: layers of a sector (stops at the first
/// layer without a texture or alpha map; a fully opaque layer with a texture without
/// bAlphaTexture silences the layers below) and the triangles of each layer: visible quads
/// where it has alpha and that no opaque layer above covers (EdgeTurnBitmap for the
/// diagonal).
pub fn sector_layers(t: &TerrainData, s: &Sector, layers: &[Layer], maps: &[Option<AlphaMap>], alpha_texture: &[bool], visibility: &[u32], edge_turn: &[u32]) -> Vec<SectorLayer> {
    // (render, map, texture has bAlphaTexture)
    let mut entries: Vec<(bool, usize)> = Vec::new();
    for (k, l) in layers.iter().enumerate() {
        if l.texture.is_none() || l.alpha_map.is_none() {
            break;
        }
        let Some(map) = maps.get(k).and_then(|m| m.as_ref()) else { break };
        let mut all_zero = true;
        let mut opaque = !alpha_texture.get(k).copied().unwrap_or(false);
        'scan: for y in 0..=s.quads_y {
            for x in 0..=s.quads_x {
                let a = t.layer_alpha(s.offset_x + x, s.offset_y + y, map);
                if a != 0xFF {
                    opaque = false;
                }
                if a != 0 {
                    all_zero = false;
                }
                if !all_zero && !opaque {
                    break 'scan;
                }
            }
        }
        if opaque {
            for e in entries.iter_mut() {
                e.0 = false;
            }
        }
        entries.push((!all_zero, k));
    }
    let n = entries.len();
    let mut out: Vec<Option<Vec<u32>>> = vec![None; n];
    let w = s.quads_x + 1;
    for li in (0..n).rev() {
        if !entries[li].0 {
            continue;
        }
        let map = maps[entries[li].1].as_ref().unwrap();
        let mut idx: Vec<u32> = Vec::new();
        for y in 0..s.quads_y {
            for x in 0..s.quads_x {
                let (gx, gy) = (s.offset_x + x, s.offset_y + y);
                let g = sector_global_vertex(t.heightmap_x, s.offset_x, s.offset_y, x, y);
                let visible = TerrainData::bit(visibility, g);
                let v00 = (w * y + x) as u32;
                let v01 = (w * (y + 1) + x) as u32;
                let (v10, v11) = (v00 + 1, v01 + 1);
                let a00 = t.layer_alpha(gx, gy, map);
                let a10 = t.layer_alpha(gx + 1, gy, map);
                let a11 = t.layer_alpha(gx + 1, gy + 1, map);
                let a01 = t.layer_alpha(gx, gy + 1, map);
                let mut covered = false;
                if visible {
                    for &(on, k) in &entries[li + 1..] {
                        let Some(m2) = maps[k].as_ref() else { continue };
                        if on
                            && t.layer_alpha(gx, gy, m2) == 0xFF
                            && t.layer_alpha(gx + 1, gy, m2) == 0xFF
                            && t.layer_alpha(gx + 1, gy + 1, m2) == 0xFF
                            && t.layer_alpha(gx, gy + 1, m2) == 0xFF
                            && !alpha_texture.get(k).copied().unwrap_or(false)
                        {
                            covered = true;
                            break;
                        }
                    }
                }
                if !visible || covered {
                    continue;
                }
                if !TerrainData::bit(edge_turn, g) {
                    if a00 != 0 || a11 != 0 || a01 != 0 {
                        idx.extend([v00, v01, v11]);
                    }
                    if a00 != 0 || a10 != 0 || a11 != 0 {
                        idx.extend([v00, v11, v10]);
                    }
                } else {
                    if a00 != 0 || a10 != 0 || a01 != 0 {
                        idx.extend([v00, v01, v10]);
                    }
                    if a01 != 0 || a10 != 0 || a11 != 0 {
                        idx.extend([v01, v11, v10]);
                    }
                }
            }
        }
        if idx.is_empty() {
            entries[li].0 = false;
        } else {
            out[li] = Some(idx);
        }
    }
    entries.iter().zip(out).filter(|(e, _)| e.0).filter_map(|(e, i)| i.map(|indices| SectorLayer { layer: e.1, indices })).collect()
}

/// FCoords::Inverse: transposed axes, origin −(O transformed by the axes).
fn coords_inverse(c: &Coords) -> Coords {
    let t = transform_vector(c, c.origin);
    Coords { origin: [-t[0], -t[1], -t[2]], x: [c.x[0], c.y[0], c.z[0]], y: [c.x[1], c.y[1], c.z[1]], z: [c.x[2], c.y[2], c.z[2]] }
}

/// (v · X, v · Y, v · Z).
fn transform_vector(c: &Coords, v: [f32; 3]) -> [f32; 3] {
    [v[2] * c.x[2] + v[1] * c.x[1] + v[0] * c.x[0], v[2] * c.y[2] + v[1] * c.y[1] + v[0] * c.y[0], v[2] * c.z[2] + v[1] * c.z[1] + v[0] * c.z[0]]
}

/// `a` expressed in frame `b` (origin through, axes through).
fn coords_mul(a: &Coords, b: &Coords) -> Coords {
    let d = [a.origin[0] - b.origin[0], a.origin[1] - b.origin[1], a.origin[2] - b.origin[2]];
    Coords { origin: transform_vector(b, d), x: transform_vector(b, a.x), y: transform_vector(b, a.y), z: transform_vector(b, a.z) }
}

/// ATerrainInfo::Render: world → texture coordinates matrix of a layer,
/// rows (FCoords::Matrix).
pub fn layer_matrix(t: &TerrainData, l: &Layer) -> [f32; 16] {
    // __ftol: truncation.
    let rot = l.rotation as i32;
    let mut c = coords::div_rotator(&coords::UNIT, [0, rot, 0]);
    c = coords_mul(&c, &coords_inverse(&t.to_heightmap));
    let su = 1.0 / l.uscale;
    c.x = c.x.map(|v| v * su);
    let sv = 1.0 / l.vscale;
    c.y = c.y.map(|v| v * sv);
    let pan = transform_vector(&t.to_world, [l.upan * l.uscale, l.vpan * l.vscale, 0.0]);
    // Machine-code addition order (pan + origin).
    c.origin = [pan[0] + c.origin[0], pan[1] + c.origin[1], pan[2] + c.origin[2]];
    match l.map_axis {
        1 => {
            // TEXMAPAXIS_XZ: origin (x, y, z) → (y, z, x); X: x ↔ z;
            // Y: x ↔ y (Y.x and Y.y exchanged); Z: y ↔ z.
            let o = c.origin;
            c.origin = [o[1], o[2], o[0]];
            c.x.swap(0, 2);
            c.y.swap(0, 1);
            c.z.swap(1, 2);
        }
        2 => {
            // Origin, X, Z: x ↔ z.
            c.origin.swap(0, 2);
            c.x.swap(0, 2);
            c.z.swap(0, 2);
        }
        _ => {}
    }
    let c2 = coords::div_rotator(&coords::UNIT, l.layer_rotation);
    c = coords_mul(&c, &c2);
    let (o, x, y, z) = (c.origin, c.x, c.y, c.z);
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
        nx * x[0] + ny * x[1] + nz * x[2],
        nx * y[0] + ny * y[1] + nz * y[2],
        nx * z[0] + ny * z[1] + nz * z[2],
        1.0,
    ]
}

#[cfg(test)]
mod tests_texture_map_axis {
    use super::*;

    fn neutral_terrain() -> TerrainData {
        TerrainData {
            heightmap_x: 0,
            heightmap_y: 0,
            vertices: Vec::new(),
            to_world: coords::UNIT,
            to_heightmap: coords::UNIT,
            sectors_x: 0,
            sectors_y: 0,
            sectors: Vec::new(),
            sounds: [0; 5],
            noise_loudness: 0.0,
        }
    }

    /// TEXMAPAXIS_XZ (ATerrainInfo::Render): U comes from Z and V from X. Measured in
    /// the original (D3D log of PRock04b, view 37, layer UScale 384 / VScale 240, texture matrix brought
    /// back to world space): u = z / 384, v = x / 240. The former port exchanged Y.x and Y.z and took V from Y.
    #[test]
    fn axis_xz_u_from_z_v_from_x() {
        let l = Layer { uscale: 384.0, vscale: 240.0, map_axis: 1, ..Default::default() };
        let m = layer_matrix(&neutral_terrain(), &l);
        let u = [m[0], m[4], m[8]];
        let v = [m[1], m[5], m[9]];
        assert_eq!(u, [0.0, 0.0, 1.0 / 384.0], "u = {u:?}");
        assert_eq!(v, [1.0 / 240.0, 0.0, 0.0], "v = {v:?}");
    }

    /// TEXMAPAXIS_YZ: U from Z, V from Y (PRock04b log: u = z / 768, v = y / 480).
    #[test]
    fn axis_yz_u_from_z_v_from_y() {
        let l = Layer { uscale: 768.0, vscale: 480.0, map_axis: 2, ..Default::default() };
        let m = layer_matrix(&neutral_terrain(), &l);
        assert_eq!([m[0], m[4], m[8]], [0.0, 0.0, 1.0 / 768.0]);
        assert_eq!([m[1], m[5], m[9]], [0.0, 1.0 / 480.0, 0.0]);
    }
}