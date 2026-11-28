//! Map assembly: StaticMesh actors placed in the world.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::model::Model;
use crate::static_mesh::StaticMesh;
use openxiii_package::library::Library;
use openxiii_package::{ObjectData, ObjectRef, Package, Value};

/// Instance of a StaticMesh in the map.
#[derive(Debug, Clone)]
pub struct MeshActor {
    pub actor: String,
    pub mesh: String,
    pub location: [f32; 3],
    /// Pitch, Yaw, Roll in Unreal units (65536 = 360°).
    pub rotation: [i32; 3],
    pub scale: [f32; 3],
    pub pre_pivot: [f32; 3],
    /// The actor blocks movement (bCollideActors and bBlockActors not disabled).
    /// Flags read from the map only, without the class defaults; the VM
    /// is authoritative.
    pub blocks: bool,
}

fn float(o: &ObjectData, p: &Package, name: &str) -> Option<f32> {
    match o.get(p, name)? {
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

fn vector(o: &ObjectData, p: &Package, name: &str) -> Option<[f32; 3]> {
    match o.get(p, name)? {
        Value::Vector(v) => Some(*v),
        _ => None,
    }
}

/// ULevelBase::Actors (ULevelBase::Serialize, after the Level object's properties):
/// two 32-bit integers (count, capacity) then one compact reference per slot, null (0) for an
/// empty slot, positive for an export (index + 1), negative for an import. Raw values.
pub fn actors_raw(package: &Package, data: &[u8], export: usize) -> Option<Vec<i32>> {
    let end = (package.exports[export].serial_offset + package.exports[export].serial_size) as usize;
    let od = package.object_data(data, export).ok()?;
    let mut r = openxiii_package::Reader::new(&data[..end], od.end);
    let n = r.i32().ok()?;
    let _max = r.i32().ok()?;
    (0..n.max(0)).map(|_| r.compact().ok()).collect()
}

/// The actor array read last by ULevel::Serialize (licensee >= 0x32: at ULevel+0x1124) that
/// ULevel::CleanLightActors walks on load: raw references like `actors_raw`. Full read of the native part:
/// ULevelBase (Actors, FURL: Protocol, Host, Map, Portal, Op, Port, Valid), Model, TimeSeconds, one reference (+0xF0), 16 references
/// (+0x90), the string-pair table (+0xD8, version > 0x3E), then this array.
pub fn light_actors_raw(package: &Package, data: &[u8], export: usize) -> Option<Vec<i32>> {
    let end = (package.exports[export].serial_offset + package.exports[export].serial_size) as usize;
    let od = package.object_data(data, export).ok()?;
    let mut r = openxiii_package::Reader::new(&data[..end], od.end);
    let n = r.i32().ok()?;
    let _max = r.i32().ok()?;
    for _ in 0..n.max(0) {
        r.compact().ok()?;
    }
    read_url(&mut r)?;
    r.compact().ok()?; // Model
    r.i32().ok()?; // TimeSeconds (float)
    r.compact().ok()?;
    for _ in 0..16 {
        r.compact().ok()?;
    }
    let pairs = r.compact().ok()?;
    for _ in 0..pairs.max(0) {
        r.string().ok()?;
        r.string().ok()?;
    }
    let k = r.compact().ok()?;
    let refs: Vec<i32> = (0..k.max(0)).map(|_| r.compact().ok()).collect::<Option<_>>()?;
    (r.pos == end).then_some(refs)
}

/// A level's FURL (ULevelBase::URL, operator<<: Protocol, Host, Map, Portal, Op (TArray<FString>, <<), Port, Valid).
#[derive(Debug, Clone, PartialEq)]
pub struct LevelUrl {
    pub protocol: String,
    pub host: String,
    pub map: String,
    pub portal: String,
    pub op: Vec<String>,
    pub port: i32,
    pub valid: i32,
}

fn read_url(r: &mut openxiii_package::Reader) -> Option<LevelUrl> {
    let (protocol, host, map, portal) = (r.string().ok()?, r.string().ok()?, r.string().ok()?, r.string().ok()?);
    let ops = r.compact().ok()?;
    let op = (0..ops.max(0)).map(|_| r.string().ok()).collect::<Option<Vec<_>>>()?;
    let (port, valid) = (r.i32().ok()?, r.i32().ok()?);
    Some(LevelUrl { protocol, host, map, portal, op, port, valid })
}

/// The URL saved in the level (after ULevelBase::Actors).
pub fn level_url(package: &Package, data: &[u8], export: usize) -> Option<LevelUrl> {
    let end = (package.exports[export].serial_offset + package.exports[export].serial_size) as usize;
    let od = package.object_data(data, export).ok()?;
    let mut r = openxiii_package::Reader::new(&data[..end], od.end);
    let n = r.i32().ok()?;
    let _max = r.i32().ok()?;
    for _ in 0..n.max(0) {
        r.compact().ok()?;
    }
    read_url(&mut r)
}

/// Map exports stored in ULevelBase::Actors (`actors_raw`): the game's only actors.
/// An actor deleted in the editor remains an export (bDeleteMe, Hual01b: TerrainInfo0,
/// SkyZoneInfo1) but its slot is empty; UGameEngine::LoadMap leaves it to the garbage collector and
/// AllActors never sees it. None without a readable Level object (everything is kept).
pub fn level_actor_exports(map: &Package, data: &[u8]) -> Option<std::collections::HashSet<usize>> {
    Some(level_actor_order(map, data)?.into_iter().collect())
}

/// The exports of ULevelBase::Actors in array order (that of AllActors), empty slots and
/// imports removed.
pub fn level_actor_order(map: &Package, data: &[u8]) -> Option<Vec<usize>> {
    let e = (0..map.exports.len()).find(|&i| map.export_class(i).eq_ignore_ascii_case("Level"))?;
    Some(actors_raw(map, data, e)?.into_iter().filter(|&x| x > 0).map(|x| (x - 1) as usize).collect())
}

/// Lists the map's actors that display a StaticMesh.
pub fn mesh_actors(map_name: &str, map: &Package, data: &[u8]) -> Vec<MeshActor> {
    let mut actors = Vec::new();
    let level = level_actor_exports(map, data);
    for i in 0..map.exports.len() {
        if map.export_class(i) == "Class" || map.exports[i].serial_size == 0 || level.as_ref().is_some_and(|l| !l.contains(&i)) {
            continue;
        }
        let Ok(o) = map.object_data(data, i) else {
            continue;
        };
        let Some(Value::Object(mesh)) = o.get(map, "StaticMesh") else {
            continue;
        };
        if *mesh == 0 {
            continue;
        }
        let Some(location) = vector(&o, map, "Location") else {
            continue;
        };
        let mesh_path = match ObjectRef::from(*mesh) {
            ObjectRef::Export(_) => format!("{map_name}.{}", map.object_path((*mesh).into())),
            _ => map.object_path((*mesh).into()),
        };
        let rotation = match o.get(map, "Rotation") {
            Some(Value::Rotator(r)) => *r,
            _ => [0; 3],
        };
        let s = float(&o, map, "DrawScale").unwrap_or(1.0);
        let s3 = vector(&o, map, "DrawScale3D").unwrap_or([1.0; 3]);
        actors.push(MeshActor {
            actor: map.object_path(ObjectRef::Export(i)),
            mesh: mesh_path,
            location,
            rotation,
            scale: [s * s3[0], s * s3[1], s * s3[2]],
            pre_pivot: vector(&o, map, "PrePivot").unwrap_or([0.0; 3]),
            blocks: !matches!(o.get(map, "bCollideActors"), Some(Value::Bool(false))) && !matches!(o.get(map, "bBlockActors"), Some(Value::Bool(false))),
        });
    }
    actors
}

/// Unreal rotation matrix (FRotationMatrix), row-vector convention: v' = v · M.
pub fn rotation_matrix(r: [i32; 3]) -> [[f32; 3]; 3] {
    let angle = |u: i32| (u as f64) * std::f64::consts::PI * 2.0 / 65536.0;
    let (sp, cp) = angle(r[0]).dsin_cos();
    let (sy, cy) = angle(r[1]).dsin_cos();
    let (sr, cr) = angle(r[2]).dsin_cos();
    let m = [[cp * cy, cp * sy, sp], [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp], [-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp]];
    m.map(|row| row.map(|v| v as f32))
}

impl MeshActor {
    /// LocalToWorld (lighting::local_to_world), DrawScale already in `scale`.
    pub fn local_to_world(&self) -> [[f32; 4]; 4] {
        crate::lighting::local_to_world(self.rotation, self.scale, 1.0, self.location, self.pre_pivot)
    }

    pub fn transform(&self, v: [f32; 3]) -> [f32; 3] {
        self.transform_with(&rotation_matrix(self.rotation), v)
    }

    /// `transform` with the rotation matrix already computed (`rotation_matrix(self.rotation)`),
    /// so its sines and cosines are not recomputed for every vertex.
    pub fn transform_with(&self, m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
        let p = [0, 1, 2].map(|k| (v[k] - self.pre_pivot[k]) * self.scale[k]);
        [0, 1, 2].map(|k| p[0] * m[0][k] + p[1] * m[1][k] + p[2] * m[2][k] + self.location[k])
    }

    /// Actor-to-world normal: inverse transpose of `transform` (inverse scale,
    /// then rotation), not normalised.
    pub fn transform_normal(&self, n: [f32; 3]) -> [f32; 3] {
        self.transform_normal_with(&rotation_matrix(self.rotation), n)
    }

    /// `transform_normal` with the rotation matrix already computed.
    pub fn transform_normal_with(&self, m: &[[f32; 3]; 3], n: [f32; 3]) -> [f32; 3] {
        let p = [0, 1, 2].map(|k| if self.scale[k] != 0.0 { n[k] / self.scale[k] } else { n[k] });
        [0, 1, 2].map(|k| p[0] * m[0][k] + p[1] * m[1][k] + p[2] * m[2][k])
    }
}

/// Assembly result: OBJ of the whole map and statistics.
pub struct WorldObj {
    pub obj: String,
    pub placed: usize,
    pub missing: Vec<String>,
    pub triangles: usize,
    /// Next (1-based) vertex and texture coordinate numbers in the OBJ.
    pub next_vertex: usize,
    pub next_uv: usize,
}

/// Builds a single OBJ (Y up) with every placed StaticMesh.
pub fn world_obj(library: &mut Library, actors: &[MeshActor]) -> WorldObj {
    let mut obj = String::new();
    let (mut base, mut base_uv, mut placed, mut triangles) = (1usize, 1usize, 0, 0);
    let mut missing = Vec::new();
    for actor in actors {
        let Some((loaded, index)) = library.resolve(&actor.mesh) else {
            missing.push(actor.mesh.clone());
            continue;
        };
        let package = &loaded.package;
        let Ok(mesh) = StaticMesh::read(package, &loaded.data, index) else {
            missing.push(actor.mesh.clone());
            continue;
        };
        let materials: Vec<String> = mesh.materials.iter().map(|&m| if m == 0 { "none".into() } else { qualified(&loaded.name, package, m) }).collect();
        obj += &format!("o {}\n", actor.actor);
        for &p in &mesh.positions {
            let w = actor.transform(p);
            obj += &format!("v {} {} {}\n", w[0], w[2], w[1]);
        }
        let uv = mesh.uvs.first();
        if let Some(uv) = uv {
            for t in uv {
                obj += &format!("vt {} {}\n", t[0], 1.0 - t[1]);
            }
        }
        // A negative scale flips the triangles: restore the order.
        let mirrored = actor.scale.iter().filter(|s| **s < 0.0).count() % 2 == 1;
        for (s, section) in mesh.sections.iter().enumerate() {
            obj += &format!("usemtl {}\n", materials.get(s).map_or("none", String::as_str));
            for t in mesh.section_triangles(section) {
                let f = |i: u16| {
                    let (v, t) = (base + i as usize, base_uv + i as usize);
                    if uv.is_some() {
                        format!("{v}/{t}")
                    } else {
                        v.to_string()
                    }
                };
                let (a, b, c) = if mirrored { (t[0], t[1], t[2]) } else { (t[0], t[2], t[1]) };
                obj += &format!("f {} {} {}\n", f(a), f(b), f(c));
                triangles += 1;
            }
        }
        base += mesh.positions.len();
        if uv.is_some() {
            base_uv += mesh.positions.len();
        }
        placed += 1;
    }
    WorldObj { obj, placed, missing, triangles, next_vertex: base, next_uv: base_uv }
}

/// Full path of a referenced object, prefixed with the package for a local export.
fn qualified(package_name: &str, package: &Package, r: i32) -> String {
    match ObjectRef::from(r) {
        ObjectRef::Export(_) => format!("{package_name}.{}", package.object_path(r.into())),
        _ => package.object_path(r.into()),
    }
}

/// Size (USize, VSize) of a texture, 256 by default (shaders, not found).
pub fn texture_size(library: &mut Library, path: &str) -> [f32; 2] {
    let Some((loaded, index)) = library.resolve(path) else {
        return [256.0; 2];
    };
    let Ok(o) = loaded.package.object_data(&loaded.data, index) else {
        return [256.0; 2];
    };
    let get = |name| match o.get(&loaded.package, name) {
        Some(Value::Int(v)) if *v > 0 => *v as f32,
        _ => 256.0,
    };
    [get("USize"), get("VSize")]
}

/// Adds the map's BSP geometry to an OBJ (vertices numbered from `base`).
pub fn bsp_obj(library: &mut Library, map_name: &str, map: &Package, model: &Model, base: usize, base_uv: usize) -> (String, usize) {
    let mut obj = String::from("o BSP\n");
    let mut sizes = std::collections::HashMap::new();
    let (mut v, mut t, mut triangles) = (base, base_uv, 0);
    for (surf_index, poly) in model.polygons() {
        let surf = &model.surfs[surf_index];
        let material = if surf.material == 0 { "none".to_string() } else { qualified(map_name, map, surf.material) };
        let size = *sizes.entry(material.clone()).or_insert_with(|| texture_size(library, &material));
        obj += &format!("usemtl {material}\n");
        for p in &poly {
            let uv = model.texel(surf, *p);
            obj += &format!("v {} {} {}\nvt {} {}\n", p[0], p[2], p[1], uv[0] / size[0], 1.0 - uv[1] / size[1]);
        }
        // Fan oriented so the OBJ normal (Y/Z swapped) follows the surface plane.
        let q: Vec<[f32; 3]> = poly.iter().map(|p| [p[0], p[2], p[1]]).collect();
        let (e1, e2) = ([0, 1, 2].map(|i| q[1][i] - q[0][i]), [0, 1, 2].map(|i| q[2][i] - q[0][i]));
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let expected = [surf.plane[0], surf.plane[2], surf.plane[1]];
        let keep = n[0] * expected[0] + n[1] * expected[1] + n[2] * expected[2] >= 0.0;
        for k in 1..poly.len() - 1 {
            let (a, b, c) = if keep { (0, k, k + 1) } else { (0, k + 1, k) };
            obj += &format!("f {}/{} {}/{} {}/{}\n", v + a, t + a, v + b, t + b, v + c, t + c);
            triangles += 1;
        }
        v += poly.len();
        t += poly.len();
    }
    (obj, triangles)
}

/// Bits of a native u32 array (compact n, then n × u32), bit i = word i/32, bit i%32.
fn bitmap(raw: &[u8]) -> Option<Vec<u32>> {
    let mut r = openxiii_package::Reader::new(raw, 0);
    let n = r.compact().ok()?;
    (0..n).map(|_| r.u32().ok()).collect()
}

/// The map's terrains (TerrainInfo) as OBJ: grid of the G16 height map,
/// vertex (x, y) = Location + ((x - L/2)·Scale.X, (y - H/2)·Scale.Y, (h - 32768)·Scale.Z/256).
/// Quads whose QuadVisibilityBitmap bit is 0 are skipped.
pub fn terrain_obj(library: &mut Library, map_name: &str, map: &Package, data: &[u8], base: usize) -> (String, usize, usize) {
    let mut obj = String::new();
    let (mut v, mut triangles) = (base, 0);
    let level = level_actor_exports(map, data);
    for i in 0..map.exports.len() {
        if map.export_class(i) != "TerrainInfo" || level.as_ref().is_some_and(|l| !l.contains(&i)) {
            continue;
        }
        let Ok(o) = map.object_data(data, i) else {
            continue;
        };
        let Some(Value::Object(height_ref)) = o.get(map, "TerrainMap") else {
            continue;
        };
        let height_path = qualified(map_name, map, *height_ref);
        let Some((loaded, index)) = library.resolve(&height_path) else {
            continue;
        };
        let Ok(texture) = crate::texture::Texture::read(&loaded.package, &loaded.data, index) else {
            continue;
        };
        let Some(mip) = texture.mips.first() else {
            continue;
        };
        let (w, h) = (mip.width as usize, mip.height as usize);
        let heights: Vec<f32> = match texture.format {
            crate::texture::Format::G16 => mip.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as f32).collect(),
            _ => mip.data.iter().map(|&b| b as f32 * 256.0).collect(),
        };
        if heights.len() < w * h {
            continue;
        }
        let scale = vector(&o, map, "TerrainScale").unwrap_or([64.0, 64.0, 64.0]);
        let location = vector(&o, map, "Location").unwrap_or([0.0; 3]);
        let visible = match o.get(map, "QuadVisibilityBitmap") {
            Some(Value::Raw(raw)) => bitmap(raw),
            _ => None,
        };
        let turned = match o.get(map, "EdgeTurnBitmap") {
            Some(Value::Raw(raw)) => bitmap(raw),
            _ => None,
        };
        let bit =
            |bits: &Option<Vec<u32>>, q: usize, default: bool| bits.as_ref().and_then(|b| b.get(q / 32)).map_or(default, |word| word >> (q % 32) & 1 == 1);
        obj += &format!("o {}\nusemtl terrain\n", map.object_path(ObjectRef::Export(i)));
        for y in 0..h {
            for x in 0..w {
                let p = [
                    location[0] + (x as f32 - (w / 2) as f32) * scale[0],
                    location[1] + (y as f32 - (h / 2) as f32) * scale[1],
                    location[2] + (heights[y * w + x] - 32768.0) * scale[2] / 256.0,
                ];
                obj += &format!("v {} {} {}\n", p[0], p[2], p[1]);
            }
        }
        for y in 0..h - 1 {
            for x in 0..w - 1 {
                let q = y * w + x;
                if !bit(&visible, q, true) {
                    continue;
                }
                let (a, b, c, d) = (v + q, v + q + 1, v + q + w, v + q + w + 1);
                // Order adapted to the Y/Z swap (normals pointing up).
                let tris = if bit(&turned, q, false) { [[a, b, d], [a, d, c]] } else { [[a, b, c], [b, d, c]] };
                for t in tris {
                    obj += &format!("f {} {} {}\n", t[0], t[1], t[2]);
                    triangles += 1;
                }
            }
        }
        v += w * h;
    }
    (obj, triangles, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::{local_to_world, transform_point};

    /// The static geometry (world::add_static_meshes, MeshActor::local_to_world) and the VM's projector
    /// triangles (projector_local_to_world: Rotation, DrawScale3D, DrawScale, Location,
    /// PrePivot) place each vertex at bit-identical positions: otherwise the shadow, drawn without
    /// SetZBias, z-fights with the floor.
    #[test]
    fn fixed_vertices_as_projectors() {
        let mut s = 0x1234_5678u32;
        let mut r = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            s
        };
        let mut f = |a: f32| (r() >> 8) as f32 / (1u32 << 24) as f32 * 2.0 * a - a;
        let mut old_diff = 0;
        for k in 0..400 {
            let rot = [(f(1.0) * 40000.0) as i32, [0, 16384, -16384, 32768][k % 4] + if k % 3 == 0 { 0 } else { (f(1.0) * 30000.0) as i32 }, (f(1.0) * 40000.0) as i32];
            let rot = if k < 40 { [0, rot[1], 0] } else { rot };
            let s3 = [1.0 + f(0.9), 1.0 + f(0.9), 1.0 + f(0.9)];
            let ds = 1.0 + f(0.5);
            let loc = [f(4000.0), f(4000.0), f(1000.0)];
            let pp = if k % 2 == 0 { [0.0; 3] } else { [f(50.0), f(50.0), f(50.0)] };
            let actor = MeshActor { actor: String::new(), mesh: String::new(), location: loc, rotation: rot, scale: [ds * s3[0], ds * s3[1], ds * s3[2]], pre_pivot: pp, blocks: false };
            let vm = local_to_world(rot, s3, ds, loc, pp);
            let fixed = actor.local_to_world();
            for _ in 0..8 {
                let v = [f(800.0), f(800.0), f(300.0)];
                assert_eq!(transform_point(&fixed, v), transform_point(&vm, v), "trial {k}: rotation {rot:?}");
                if actor.transform(v) != transform_point(&vm, v) {
                    old_diff += 1;
                }
            }
        }
        // Previous placement (sine in f64): rounding differences, the cause of the defect.
        assert!(old_diff > 0);
    }
}