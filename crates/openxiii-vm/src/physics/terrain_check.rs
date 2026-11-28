//! Traces against the terrain: ATerrainInfo::LineCheck and LineCheckWithQuad,
//! on the serialized vertices (Vertices, ATerrainInfo::Serialize) in the machine code's order
//! of operations (x87 at 24-bit precision). The contact's material comes from the
//! table of ATerrainInfo::ComputeQuadMaterials.
//!
//! - LineCheck transforms Start, End and Extent into the heightmap (ToHeightmap), widens the path
//!   by Extent.X (in the map) along its direction and walks the quads column by column
//!   (or row by row), stopping once it has gone more than Extent.X past a hit quad.
//! - LineCheckWithQuad splits the quad according to EdgeTurnBitmap and bInverted, raises the terrain by
//!   Extent.Z and accepts a point of the triangle's plane within Extent.X of each edge. The
//!   contact moves back by 0.5 / |End − Start| in time.

use crate::physics::{CheckResult, V};
use crate::value::Value;
use crate::{ObjId, Vm};
use openxiii_assets::coords::Coords;

/// Collision data of a TerrainInfo of ULevel.Terrains (ULevel + 0x110C).
pub(crate) struct TerrainCol {
    actor: ObjId,
    hx: i32,
    hy: i32,
    vertices: Vec<V>,
    to_heightmap: Coords,
    /// TerrainMap non-null (otherwise LineCheck returns nothing).
    has_map: bool,
    /// Real G16 object created from the legacy height words, retained with its owning terrain.
    pub(crate) generated_map: Option<(String, openxiii_assets::terrain_postload::TerrainMap)>,
    visible: Vec<u32>,
    edge_turn: Vec<u32>,
    inverted: bool,
    /// Layer of each quad (QuadMaterialTable, one nibble per quad in the engine).
    quad_material: Vec<u8>,
    /// Label (index in `collision_materials`) of Layers[i].Texture.
    layer_tag: Vec<Option<u32>>,
}

pub(crate) fn bit(bitmap: &[u32], i: i32) -> bool {
    i >= 0 && bitmap.get((i >> 5) as usize).is_some_and(|w| w & (1 << (i & 31)) != 0)
}

/// FCoords: TransformPointBy, ((dX·Ax + dY·Ay) + dZ·Az) with d = P − Origin.
pub(crate) fn transform_point(c: &Coords, p: V) -> V {
    let d = [p[0] - c.origin[0], p[1] - c.origin[1], p[2] - c.origin[2]];
    [(d[0] * c.x[0] + d[1] * c.x[1]) + d[2] * c.x[2], (d[0] * c.y[0] + d[1] * c.y[1]) + d[2] * c.y[2], (d[0] * c.z[0] + d[1] * c.z[1]) + d[2] * c.z[2]]
}

/// TransformVectorBy: same product without the origin.
fn transform_vector(c: &Coords, v: V) -> V {
    [(v[0] * c.x[0] + v[1] * c.x[1]) + v[2] * c.x[2], (v[0] * c.y[0] + v[1] * c.y[1]) + v[2] * c.y[2], (v[0] * c.z[0] + v[1] * c.z[1]) + v[2] * c.z[2]]
}

/// FVector::SizeSquared: (X² + Y²) + Z².
fn size_sq(a: V) -> f32 {
    (a[0] * a[0] + a[1] * a[1]) + a[2] * a[2]
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Normal of FPlane(A, B, C): (B − A) ^ (C − A) normalized
/// ((X² + Y²) + Z² ≥ 1e-8 in double, appRSqrt), zero otherwise.
fn plane_normal(a: V, b: V, c: V) -> V {
    let (u, w) = (sub(b, a), sub(c, a));
    let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    let sq = (n[0] * n[0] + n[1] * n[1]) + n[2] * n[2];
    if (sq as f64) < 1e-8 {
        return [0.0; 3];
    }
    let k = crate::natives::app_rsqrt(sq);
    [k * n[0], k * n[1], k * n[2]]
}

/// FPlane(Point, Normal).W: (P.Y·N.Y + P.Z·N.Z) + P.X·N.X.
fn plane_w(p: V, n: V) -> f32 {
    (p[1] * n[1] + p[2] * n[2]) + p[0] * n[0]
}

/// Signed distance to the plane, as LineCheckWithQuad writes it: (I.Y·N.Y + I.Z·N.Z) + I.X·N.X − W.
fn plane_dot(i: V, n: V, w: f32) -> f32 {
    ((i[1] * n[1] + i[2] * n[2]) + i[0] * n[0]) - w
}

/// FLinePlaneIntersection from P1 to P2: t = (W − ((P1.X·N.X + N.Z·P1.Z) + N.Y·P1.Y))
/// / ((dZ·N.Z + dY·N.Y) + dX·N.X), P1 + d × t.
fn line_plane(p1: V, p2: V, n: V, w: f32) -> V {
    let d = sub(p2, p1);
    let t = (w - ((p1[0] * n[0] + n[2] * p1[2]) + n[1] * p1[1])) / ((d[2] * n[2] + d[1] * n[1]) + d[0] * n[0]);
    [d[0] * t + p1[0], d[1] * t + p1[1], d[2] * t + p1[2]]
}

/// Plane of an edge: (Sign × Edge) ^ N, through `p`.
fn edge_dist(i: V, edge: V, sign: f32, n: V, p: V) -> f32 {
    let a = [sign * edge[0], sign * edge[1], sign * edge[2]];
    let c = [a[1] * n[2] - a[2] * n[1], a[2] * n[0] - n[2] * a[0], a[0] * n[1] - a[1] * n[0]];
    plane_dot(i, c, plane_w(p, c))
}

/// Clamp(v, lo, hi) (NaN gives hi).
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v < hi {
        v
    } else {
        hi
    }
}

/// _ftol: truncation towards zero, then clamped to [0, max] as in LineCheck.
fn ftol_clamp(v: f32, max: i32) -> i32 {
    let i = v as i32;
    if i < 0 {
        0
    } else if i >= max {
        max
    } else {
        i
    }
}

impl TerrainCol {
    pub(crate) fn actor(&self) -> ObjId {
        self.actor
    }

    /// ATerrainInfo::LineCheckWithQuad: the two triangles of quad (x, y). Keeps the
    /// contact closest to Start already in `result`; true if a triangle was kept.
    #[allow(clippy::too_many_arguments)]
    fn quad_check(&self, x: i32, y: i32, result: &mut CheckResult, end: V, start: V, extent: V, all_quads: bool) -> bool {
        if (!all_quads && !bit(&self.visible, self.hx * y + x)) || x >= self.hx - 1 || x < 0 || y >= self.hy - 1 || y < 0 {
            return false;
        }
        let s = if self.inverted { -1.0f32 } else { 1.0 };
        let edge = bit(&self.edge_turn, self.hx * y + x);
        // Terrain raised by Extent.Z (upwards, downwards if it is inverted).
        let dz = -extent[2] * s;
        let dxy = s * 0.0;
        let i0 = (self.hx * y + x) as usize;
        let i3 = ((y + 1) * self.hx + 1 + x) as usize;
        let at = |i: usize| {
            let v = self.vertices.get(i).copied().unwrap_or([0.0; 3]);
            [v[0] - dxy, v[1] - dxy, v[2] - dz]
        };
        let (p0, p1, p3, p2) = (at(i0), at(i0 + 1), at(i3), at(i3 - 1));
        let d = sub(end, start);
        let mut hit = false;
        // First triangle (P0, P1, P3) or, when split, (P0, P1, P2); reverse order if inverted.
        let tri = match (edge, self.inverted) {
            (true, true) => (p0, p2, p1),
            (true, false) => (p0, p1, p2),
            (false, true) => (p0, p3, p1),
            (false, false) => (p0, p1, p3),
        };
        let edges = if edge { [(sub(p1, p0), p0), (sub(p2, p1), p1), (sub(p0, p2), p2)] } else { [(sub(p1, p0), p0), (sub(p3, p1), p1), (sub(p0, p3), p3)] };
        hit |= self.tri_check(x, y, result, end, start, extent, d, tri, p0, edges, s);
        // Second triangle (P0, P3, P2) or, when split, (P2, P1, P3).
        let tri = match (edge, self.inverted) {
            (true, true) => (p2, p3, p1),
            (true, false) => (p2, p1, p3),
            (false, true) => (p0, p2, p3),
            (false, false) => (p0, p3, p2),
        };
        let edges = if edge { [(sub(p1, p2), p2), (sub(p3, p1), p1), (sub(p2, p3), p3)] } else { [(sub(p3, p0), p0), (sub(p2, p3), p3), (sub(p0, p2), p2)] };
        hit |= self.tri_check(x, y, result, end, start, extent, d, tri, p3, edges, s);
        hit
    }

    /// One triangle of LineCheckWithQuad: face turned towards the path (N·D < −1e-4), point of the plane
    /// (through `on`) on the line End → Start, closer than the contact already kept, time
    /// in [0, 1] and within Extent.X of the three edges.
    #[allow(clippy::too_many_arguments)]
    fn tri_check(&self, x: i32, y: i32, result: &mut CheckResult, end: V, start: V, extent: V, d: V, tri: (V, V, V), on: V, edges: [(V, V); 3], s: f32) -> bool {
        let n = plane_normal(tri.0, tri.1, tri.2);
        if (n[1] * d[1] + n[2] * d[2]) + d[0] * n[0] >= -1.0e-4 {
            return false;
        }
        let i = line_plane(end, start, n, plane_w(on, n));
        if result.actor.is_some() && size_sq(sub(result.location, start)) <= size_sq(sub(i, start)) {
            return false;
        }
        let r = sub(i, start);
        let t = ((r[0] * d[0] + r[1] * d[1]) + r[2] * d[2]) / ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]);
        if !(0.0..=1.0).contains(&t) {
            return false;
        }
        for (e, p) in edges {
            // NaN does not pass (the machine code's JP jump).
            if edge_dist(i, e, s, n, p).partial_cmp(&extent[0]).is_none_or(|o| o == std::cmp::Ordering::Greater) {
                return false;
            }
        }
        // Contact: Normal = N, Time = (I − Start)·D / |D|² moved back by 0.5 / |D|,
        // Location = Start + D × Time.
        result.normal = n;
        let size = (size_sq(d) as f64).sqrt() as f32;
        let t = clamp(t - 0.5 / size, 0.0, 1.0);
        result.time = t;
        result.actor = Some(self.actor);
        result.location = [start[0] + d[0] * t, start[1] + d[1] * t, start[2] + d[2] * t];
        // Non-null Owner: normal through Owner->ToWorld() (no map terrain has one).
        let layer = self.quad_material.get((self.hx * y + x) as usize).copied().unwrap_or(0) as usize;
        result.material = self.layer_tag.get(layer).copied().flatten();
        true
    }

    /// ATerrainInfo::LineCheck. `all_quads`: last parameter (0 from
    /// SingleLineCheck and MultiLineCheck: invisible quads do not block).
    pub(crate) fn line_check(&self, result: &mut CheckResult, end: V, start: V, extent: V, all_quads: bool) -> bool {
        if !self.has_map || self.vertices.is_empty() {
            return false;
        }
        let e = transform_point(&self.to_heightmap, end);
        let s = transform_point(&self.to_heightmap, start);
        let x = transform_vector(&self.to_heightmap, extent);
        let n = crate::physics::safe_normal(sub(e, s));
        let ext = x[0];
        let b = [s[0] - ext * n[0], s[1] - ext * n[1]];
        let a = [e[0] + ext * n[0], ext * n[1] + e[1]];
        if !((b[0] >= 0.0 || a[0] >= 0.0) && (b[1] >= 0.0 || a[1] >= 0.0)) {
            return false;
        }
        let (w1, h1) = ((self.hx - 1) as f32, (self.hy - 1) as f32);
        if !(b[0] <= w1 || a[0] <= w1) || !(b[1] <= h1 || a[1] <= h1) {
            return false;
        }
        result.actor = None;
        let mut found = false;
        let x0 = ftol_clamp(b[0], self.hx - 2);
        let x1 = ftol_clamp(a[0], self.hx - 2);
        if x0 == x1 {
            let y0 = ftol_clamp(b[1], self.hy - 2);
            let y1 = ftol_clamp(a[1], self.hy - 2);
            if y0 == y1 {
                return self.quad_check(x0, y0, result, end, start, extent, all_quads);
            }
            // Row by row: X = Y × slope + c, widened by Extent.X.
            let slope = (a[0] - b[0]) / (a[1] - b[1]);
            let step = if y0 <= y1 { 1 } else { -1 };
            let c = a[0] - slope * a[1];
            let mut found_at = 0;
            let mut yy = y0;
            while yy * step <= y1 * step {
                let mut lo = yy as f32 * slope + c;
                let mut hi = (yy + 1) as f32 * slope + c;
                if lo > hi {
                    std::mem::swap(&mut lo, &mut hi);
                }
                let xs = ftol_clamp(lo - ext, self.hx - 2);
                let xe = ftol_clamp(hi + ext, self.hx - 2);
                for xx in xs..=xe {
                    if self.quad_check(xx, yy, result, end, start, extent, all_quads) {
                        if !found {
                            found_at = yy;
                        }
                        found = true;
                    }
                }
                if found && ((found_at - yy).abs() as f32) > ext {
                    return found;
                }
                yy += step;
            }
        } else {
            // Column by column: Y = X × slope + c; rows bounded to HeightmapY − 1.
            let slope = (a[1] - b[1]) / (a[0] - b[0]);
            let step = if x0 <= x1 { 1 } else { -1 };
            let c = a[1] - slope * a[0];
            let mut found_at = 0;
            let mut xx = x0;
            while xx * step <= x1 * step {
                let mut lo = xx as f32 * slope + c;
                let mut hi = (xx + 1) as f32 * slope + c;
                if lo > hi {
                    std::mem::swap(&mut lo, &mut hi);
                }
                let ys = ftol_clamp(lo - ext, self.hy - 1);
                let ye = ftol_clamp(hi + ext, self.hy - 1);
                for yy in ys..=ye {
                    if self.quad_check(xx, yy, result, end, start, extent, all_quads) {
                        if !found {
                            found_at = xx;
                        }
                        found = true;
                    }
                }
                if found && ((found_at - xx).abs() as f32) > ext {
                    return found;
                }
                xx += step;
            }
        }
        found
    }
}

impl Vm {
    /// Label (index in `collision_materials`) of a material name, added if missing.
    fn terrain_material_tag(&mut self, m: &str) -> u32 {
        if self.collision_materials.is_empty() {
            self.collision_materials.push(String::new());
        }
        match self.collision_materials.iter().position(|x| x == m) {
            Some(i) => i as u32,
            None => {
                self.collision_materials.push(m.to_string());
                (self.collision_materials.len() - 1) as u32
            }
        }
    }

    /// ULevel::UpdateTerrainArrays: ULevel.Terrains, the TerrainInfos in actor
    /// order, with their native data, the bitmaps at the size of the map (PostLoad:
    /// visibility at 0xFF, split at 0 otherwise) and the quads' material table.
    pub(crate) fn prepare_terrain_collision(&mut self) {
        self.terrains = Vec::new();
        let Some(pi) = self.map_package else { return };
        let actors: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.alive(a) && self.is_a(a, "TerrainInfo")).collect();
        for a in actors {
            let name = self.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default();
            let mut loaded = {
                let lp = &self.packages[pi];
                let Some(i) = (0..lp.package.exports.len()).find(|&i| lp.package.export_class(i) == "TerrainInfo" && lp.package.object_name(openxiii_package::ObjectRef::Export(i)).eq_ignore_ascii_case(&name)) else { continue };
                let Some(library) = self.library.as_mut() else { continue };
                let Some(loaded) = openxiii_assets::terrain_postload::read(library, &lp.name, &lp.package, &lp.data, i) else { continue };
                loaded
            };
            loaded.location = self.propr(a, "Location").as_vector();
            loaded.scale = self.propr(a, "TerrainScale").as_vector();
            let bits = |vm: &Vm, n: &str| -> Vec<u32> {
                match vm.prop(a, n) { Value::Array(v) => v.iter().map(|x| x.as_int() as u32).collect(), _ => Vec::new() }
            };
            loaded.visible = bits(self, "QuadVisibilityBitmap");
            loaded.turned = bits(self, "EdgeTurnBitmap");
            loaded.material_table = bits(self, "QuadMaterialTable").into_iter().map(|x| x as u8).collect();
            let converted_legacy = !loaded.old_heightmap.is_empty();
            let mut messages = Vec::new();
            if loaded.post_load(self.library.as_mut().unwrap(), |m| messages.push(m)).is_err() { continue; }
            self.log.extend(messages);
            self.set_prop(a, "Location", Value::vector(loaded.location[0], loaded.location[1], loaded.location[2]));
            self.set_prop(a, "QuadVisibilityBitmap", Value::Array(loaded.visible.iter().map(|&x| Value::Int(x as i32)).collect()));
            self.set_prop(a, "EdgeTurnBitmap", Value::Array(loaded.turned.iter().map(|&x| Value::Int(x as i32)).collect()));
            self.set_prop(a, "QuadMaterialTable", Value::Array(loaded.material_table.iter().map(|&x| Value::Byte(x)).collect()));
            let layer_tag = loaded.layers.iter().take(16).map(|l| l.texture.as_deref().map(|p| self.terrain_material_tag(p))).collect();
            let quad_material = loaded.quad_materials();
            let has_map = loaded.terrain_map.is_some();
            let generated_map = if converted_legacy {
                let path = format!("{}.{}", self.packages[pi].name, openxiii_assets::terrain_postload::heightmap_texture_name(&name));
                self.set_prop(a, "TerrainMap", Value::Asset(path.clone()));
                loaded.terrain_map.clone().map(|map| (path, map))
            } else { None };
            let (visible, edge_turn) = (loaded.visible, loaded.turned);
            let data = loaded.data;
            let (hx, hy) = (data.heightmap_x, data.heightmap_y);
            self.terrains.push(TerrainCol {
                actor: a,
                hx,
                hy,
                vertices: data.vertices,
                to_heightmap: data.to_heightmap,
                has_map,
                generated_map,
                visible,
                edge_turn,
                inverted: self.propr(a, "bInverted").as_bool(),
                quad_material,
                layer_tag,
            });
        }
    }
}

