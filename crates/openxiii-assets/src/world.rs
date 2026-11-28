//! A map's geometry ready for the GPU: vertices, indices and per-material batches.
//!
//! Unreal coordinates are kept (left-handed, Z up, 1 unit ≈ 1.9 cm);
//! the viewer's camera does the conversion.

use crate::level::{mesh_actors, MeshActor};
use crate::model::{level_model, Model, PF_FAKE_BACKDROP, PF_INVISIBLE, PF_PORTAL};

/// PF_Mirrored (mirror surfaces, FMirrorSceneNode).
const PF_MIRRORED: u32 = 0x0800_0000;
use crate::static_mesh::StaticMesh;
use crate::texture::{Format, Texture};
use openxiii_package::library::Library;
use openxiii_package::{ObjectRef, Package, Value};
use std::collections::HashMap;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// A batch of triangles sharing a material.
#[derive(Debug, Clone)]
pub struct Batch {
    pub material: String,
    pub first_index: u32,
    pub index_count: u32,
}

/// What the engine sets before drawing a batch (in addition to the material).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct DrawState {
    /// Lightmap texture (path) passed to EnableLighting.
    pub lightmap: Option<String>,
    /// SetCullMode(CM_None): PF_TwoSided surfaces or portals.
    pub two_sided: bool,
    /// EnableLighting(UseDynamic, UseStatic): D3DRS_LIGHTING and D3DRS_COLORVERTEX.
    pub lighting: bool,
    pub color_vertex: bool,
    /// The vertex stream carries a diffuse colour.
    pub has_color: bool,
    /// D3DRS_AMBIENT (SetAmbientLight): the actor's ambient colour (R, G, B).
    pub ambient: [u8; 3],
    /// Normals of the batch in mesh space (actor batches).
    pub local_normals: bool,
    /// Zone whose fog sets before the draw (SetDistanceFog per zone
    /// for actors: Region.ZoneNumber; for BSP surfaces: zone of the node). None: the
    /// camera zone's fog.
    pub zone: Option<u8>,
}

#[derive(Debug, Default)]
pub struct World {
    /// Map name (build_world): renderer prewarm lists.
    pub map_value: String,
    pub vertices: Vec<Vertex>,
    /// Diffuse colour of each vertex (R, G, B, A).
    pub colors: Vec<[u8; 4]>,
    /// Second texture coordinates (BSP lightmaps, second UV stream of StaticMeshes).
    pub uv2: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
    /// Actor (lowercase name) owning each batch, if any.
    pub batch_owners: Vec<Option<String>>,
    /// Draw state of each batch (same order as `batches`).
    pub states: Vec<DrawState>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// StaticMeshes drawn actor by actor (cartoon or outline rendering).
    pub actor_meshes: Vec<ActorMesh>,
    /// The map's lights (UModel+0x110), initialised as by UModel::PostLoad.
    pub lights: Vec<crate::lighting::Light>,
    /// BSP nodes for UModel::PointRegion, and RootOutside.
    pub region_nodes: Vec<RegionNode>,
    pub root_outside: bool,
    /// For each zone number: fog (FLevelSceneNode::Render) and sky zone.
    pub zones: Vec<ZoneRender>,
    /// Lightmaps computed at load time (FLightManager::Cache), by texture name.
    pub runtime_lightmaps: Vec<(String, crate::lightmap::SurfaceLightmap)>,
    /// The map's BSP (lightmaps completed every frame) and the lights as
    /// serialized, with LevelInfo.Brightness, for FLight::Init every frame.
    pub model: Option<std::sync::Arc<Model>>,
    pub light_records: Vec<crate::model::LightRecord>,
    pub level_brightness: f32,
    /// Player start point (first PlayerStart), if any.
    pub player_start: Option<([f32; 3], i32)>,
    /// Blocking triangles (positions), for collision.
    pub collision: Vec<[[f32; 3]; 3]>,
    /// For each blocking triangle: index of its material in `collision_materials`.
    pub collision_tags: Vec<u32>,
    /// Materials of the blocking surfaces (0 is "none").
    pub collision_materials: Vec<String>,
    /// For each batch (same order as `batches`): index ranges per BSP node (node,
    /// first index, count), in batch order; node u32::MAX for triangles that do not
    /// come from the BSP. Empty for a batch without BSP triangles. Used to sort the scenery by
    /// visibility (only the nodes of the draw lists are drawn).
    pub bsp_node_ranges: Vec<Vec<(u32, u32, u32)>>,
    /// The map's terrains (lowercase name of the TerrainInfo actor) and the index ranges of
    /// each: (terrain, first index, count). A terrain is only drawn by the scenes that
    /// render it (terrain loop).
    pub terrain_names: Vec<String>,
    pub terrain_ranges: Vec<(usize, u32, u32)>,
    /// StaticMesh sections with a SinusModifier material, waved every frame.
    pub sine: Vec<SineSection>,
}

/// StaticMesh section whose material is a SinusModifier (SetSinusVertexStreams): its
/// vertices have their own copy in `vertices` starting at `base`, local positions in
/// `local`, put back into world space by `actor` after.
#[derive(Debug, Clone)]
pub struct SineSection {
    pub modifier: String,
    pub owner: Option<String>,
    pub base: u32,
    pub local: Vec<[f32; 3]>,
    pub actor: crate::level::MeshActor,
    /// UStaticMesh+0x17C and +0x180 (AmplitudeFadeX fade bounds).
    pub fade: [f32; 2],
}

/// BSP node for zone lookup (UModel::PointRegion).
#[derive(Debug, Clone)]
pub struct RegionNode {
    pub plane: [f32; 4],
    pub back: i16,
    pub front: i16,
    pub flags: u8,
    pub num_vertices: u8,
    pub zone: [u8; 2],
}

/// What the renderer takes from a zone (ZoneInfo, or LevelInfo).
#[derive(Debug, Clone, Default)]
pub struct ZoneRender {
    /// bDistanceFog: start, end, colour (R, G, B).
    pub fog: Option<(f32, f32, [u8; 3])>,
    /// bForceClear: UGameEngine::Draw clears the frame to DistanceFogColor (R, G, B) instead of black.
    pub clear: Option<[u8; 3]>,
    /// Sky zone (ZoneInfo.SkyZone set by LinkToSkybox): position, rotation, zone number.
    pub sky: Option<([f32; 3], [i32; 3], u8)>,
    /// LightCurveShadow | LightCurveBright << 8 and AmbientIntensity / 255 (cartoon rendering).
    pub light_curve: u32,
    pub ambient_intensity: f32,
    /// D3DRS_AMBIENT of an actor placed in the zone (FDynamicActor::Refresh: zone ambient light).
    pub ambient: [u8; 3],
    /// FGetHSV of the zone's ambient light (before FColor), for the maximum over an actor's zones.
    pub ambient_hsv: [f32; 3],
    /// bFogPerZone (XIII, ZoneInfo+0x1FC & 8): seen from this zone, sets the fog of
    /// each drawn element's zone (otherwise the camera zone's fog for everything).
    pub fog_per_zone: bool,
}

/// UModel::PointRegion: zone number of a point.
pub fn point_zone(nodes: &[RegionNode], root_outside: bool, p: [f32; 3]) -> usize {
    if nodes.is_empty() {
        return 0;
    }
    let mut outside = root_outside;
    let (mut node, mut prev, mut side) = (0i32, 0usize, 0usize);
    while node != -1 {
        let n = &nodes[node as usize];
        prev = node as usize;
        let solid = n.num_vertices != 0 && n.flags & 0x21 == 0;
        if p[0] * n.plane[0] + p[2] * n.plane[2] + p[1] * n.plane[1] - n.plane[3] < 0.0 {
            side = 0;
            outside = !(!outside || solid);
            node = n.back as i32;
        } else {
            side = 1;
            outside = !(!outside && !solid);
            node = n.front as i32;
        }
    }
    nodes[prev].zone[side] as usize
}

/// Placed StaticMesh th draws through DrawCartoonStatic: cartoon rendering
/// (UPrimitive+0x29) and/or outline (UPrimitive+0x28). Its vertices carry the mesh
/// normal (local space), like the vertex stream read by D3DDrv's shaders.
#[derive(Debug, Clone)]
pub struct ActorMesh {
    /// Sections: material, first index, index count.
    pub sections: Vec<(String, u32, u32)>,
    /// Fixed fields of FCartoonDesc (outline, cartoon, normals, factor, curve, ambient).
    pub desc: crate::cartoon::CartoonDesc,
    pub local_to_world: [[f32; 4]; 4],
    pub location: [f32; 3],
    /// |DrawScale| × max |DrawScale3D|.
    pub scale: f32,
    /// Render bounding sphere (centre, radius).
    pub sphere: [f32; 4],
    /// bSLightGroup.
    pub light_group: u8,
    pub leaf_zones: Vec<i32>,
    /// Leaf indices, to look up the lists of the current scene context.
    pub leaves: Vec<i32>,
    /// Lights that affect each of these leaves (Model::leaf_lights).
    pub leaf_lights: Vec<Vec<usize>>,
    /// Non-cartoon path (outline only): EnableLighting, per-vertex colours, ambient.
    pub state: DrawState,
    /// Negative scale: triangles flipped and SetCullMode inverted.
    pub mirrored: bool,
    /// Owning actor (lowercase name), hidden along with it.
    pub owner: Option<String>,
}

/// Decoded RGBA image of a material.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub(crate) fn qualified(package_name: &str, package: &Package, r: i32) -> String {
    match ObjectRef::from(r) {
        ObjectRef::Export(_) => format!("{package_name}.{}", package.object_path(r.into())),
        _ => package.object_path(r.into()),
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        [0.0, 0.0, 1.0]
    }
}

struct Builder {
    world: World,
    /// Indices of StaticMeshes drawn per actor, added after the batches.
    actor_indices: Vec<Vec<Vec<u32>>>,
    /// Indices sorted by material, draw state and owning actor, grouped at the end.
    groups: HashMap<(String, DrawState, Option<String>), Vec<u32>>,
    /// Draw state of the next triangles.
    state: DrawState,
    /// Non-static actor (bStatic false) of the next triangles: its batch stays separate, so it
    /// can be hidden when the actor is (picked up, destroyed).
    owner: Option<String>,
    /// BSP node of the next triangles (u32::MAX outside the BSP), and node of each triangle
    /// of the groups.
    node: u32,
    group_nodes: HashMap<(String, DrawState, Option<String>), Vec<u32>>,
}

impl Builder {
    fn triangle(&mut self, material: &str, a: u32, b: u32, c: u32) {
        let key = (material.to_string(), self.state.clone(), self.owner.clone());
        self.group_nodes.entry(key.clone()).or_default().push(self.node);
        self.groups.entry(key).or_default().extend([a, b, c]);
    }

    fn collide(&mut self, a: u32, b: u32, c: u32, material: &str) {
        let v = &self.world.vertices;
        let tri = [v[a as usize].position, v[b as usize].position, v[c as usize].position];
        self.collide_points(tri, material);
    }

    fn collide_points(&mut self, tri: [[f32; 3]; 3], material: &str) {
        if self.world.collision_materials.is_empty() {
            self.world.collision_materials.push(String::new());
        }
        let tag = match self.world.collision_materials.iter().position(|m| m == material) {
            Some(i) => i,
            None => {
                self.world.collision_materials.push(material.to_string());
                self.world.collision_materials.len() - 1
            }
        };
        self.world.collision.push(tri);
        self.world.collision_tags.push(tag as u32);
    }

    fn vertex_full(&mut self, v: Vertex, color: [u8; 4], uv2: [f32; 2]) -> u32 {
        self.world.vertices.push(v);
        self.world.colors.push(color);
        self.world.uv2.push(uv2);
        (self.world.vertices.len() - 1) as u32
    }

    fn finish(mut self) -> World {
        let mut materials: Vec<_> = self.groups.into_iter().collect();
        materials.sort_by(|a, b| a.0.cmp(&b.0));
        for ((material, state, owner), indices) in materials {
            let first_index = self.world.indices.len() as u32;
            // Consecutive ranges of triangles of the same node.
            let nodes = self.group_nodes.remove(&(material.clone(), state.clone(), owner.clone())).unwrap_or_default();
            let mut ranges: Vec<(u32, u32, u32)> = Vec::new();
            if nodes.iter().any(|&n| n != u32::MAX) {
                for (t, &n) in nodes.iter().enumerate() {
                    let at = first_index + t as u32 * 3;
                    match ranges.last_mut() {
                        Some(r) if r.0 == n && r.1 + r.2 == at => r.2 += 3,
                        _ => ranges.push((n, at, 3)),
                    }
                }
            }
            // Terrain triangles (node TERRAIN | terrain index): separate ranges.
            let (terrains, ranges): (Vec<_>, Vec<_>) = ranges.into_iter().partition(|r| r.0 != u32::MAX && r.0 & TERRAIN != 0);
            self.world.terrain_ranges.extend(terrains.into_iter().map(|(n, f, c)| ((n & !TERRAIN) as usize, f, c)));
            self.world.bsp_node_ranges.push(ranges);
            self.world.batches.push(Batch { material, first_index, index_count: indices.len() as u32 });
            self.world.states.push(state);
            self.world.batch_owners.push(owner);
            self.world.indices.extend(indices);
        }
        for (mesh, sections) in self.world.actor_meshes.iter_mut().zip(self.actor_indices) {
            for (section, indices) in mesh.sections.iter_mut().zip(sections) {
                section.1 = self.world.indices.len() as u32;
                section.2 = indices.len() as u32;
                self.world.indices.extend(indices);
            }
        }
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for v in &self.world.vertices {
            for k in 0..3 {
                min[k] = min[k].min(v.position[k]);
                max[k] = max[k].max(v.position[k]);
            }
        }
        self.world.bounds_min = min;
        self.world.bounds_max = max;
        self.world
    }
}

/// Map lighting: lights, zones and BSP leaves.
struct LevelLighting {
    model: Option<Model>,
    nodes: Vec<RegionNode>,
    lights: Vec<crate::lighting::Light>,
    level_info: Option<crate::props::Props>,
    zones: Vec<Option<crate::props::Props>>,
    brightness: f32,
}

impl LevelLighting {
    fn new(library: &mut Library, props: &mut crate::props::PropCache, map_name: &str, map: &Package, data: &[u8]) -> Self {
        let model = level_model(map, data).and_then(|i| Model::read(map, data, i).ok());
        let level_info = (0..map.exports.len())
            .find(|&i| map.export_class(i) == "LevelInfo")
            .and_then(|i| props.object(library, &qualified(map_name, map, i as i32 + 1)).cloned());
        let brightness = level_info.as_ref().map_or(1.0, |l| l.float_or("Brightness", 1.0));
        // UModel::PostLoad initialises every light (time 0).
        let mut frand = || 0.0f32;
        let lights = model.as_ref().map(|m| m.lights.iter().map(|r| crate::lighting::init_light(r, brightness, 0.0, &mut frand)).collect()).unwrap_or_default();
        let zones = model
            .as_ref()
            .map(|m| m.zones.iter().map(|z| (z.actor != 0).then(|| props.object(library, &qualified(map_name, map, z.actor)).cloned()).flatten()).collect())
            .unwrap_or_default();
        let nodes = model
            .as_ref()
            .map(|m| {
                m.nodes
                    .iter()
                    .map(|n| RegionNode { plane: n.plane, back: n.links[0], front: n.links[1], flags: n.flags, num_vertices: n.num_vertices, zone: n.zone })
                    .collect()
            })
            .unwrap_or_default();
        LevelLighting { model, nodes, lights, level_info, zones, brightness }
    }

    fn point_zone(&self, p: [f32; 3]) -> usize {
        match &self.model {
            Some(m) => point_zone(&self.nodes, m.root_outside, p),
            None => 0,
        }
    }

    /// ULevel::GetZoneActor: zone actor, or LevelInfo.
    fn zone(&self, zone: usize) -> Option<&crate::props::Props> {
        self.zones.get(zone).and_then(Option::as_ref).or(self.level_info.as_ref())
    }

    fn ambient(&self, zone: usize) -> crate::lighting::ZoneAmbient {
        match self.zone(zone) {
            Some(z) => {
                crate::lighting::ZoneAmbient { hue: z.byte("AmbientHue"), saturation: z.byte("AmbientSaturation"), brightness: z.byte("AmbientBrightness") }
            }
            None => Default::default(),
        }
    }
}

/// Placed StaticMeshes, lit as: cartoon or outline path drawn
/// actor by actor, otherwise the UStaticMeshInstance's per-vertex colours (recomputed by
/// CalculateLighting when the engine would) and D3DRS_AMBIENT from FDynamicActor::Refresh.
fn add_static_meshes(b: &mut Builder, library: &mut Library, map_name: &str, actors: &[MeshActor], level: &LevelLighting, props: &mut crate::props::PropCache) {
    use crate::lighting::{ambient_color, box_leaves, calculate_lighting, local_to_world, transform_point, zones_ambient_max};
    let mut cache: HashMap<String, Option<(StaticMesh, Vec<String>)>> = HashMap::new();
    for actor in actors {
        if !cache.contains_key(&actor.mesh) {
            let loaded = crate::mod_mesh(&actor.mesh).or_else(|| library.resolve(&actor.mesh).and_then(|(loaded, index)| {
                let mesh = StaticMesh::read(&loaded.package, &loaded.data, index).ok()?;
                let materials = mesh.materials.iter().map(|&m| if m == 0 { String::new() } else { qualified(&loaded.name, &loaded.package, m) }).collect();
                Some((mesh, materials))
            }));
            cache.insert(actor.mesh.clone(), loaded);
        }
        let Some(Some((mesh, materials))) = cache.get(&actor.mesh) else {
            continue;
        };
        let a = props.object(library, &format!("{map_name}.{}", actor.actor)).cloned();
        let flag = |name: &str| a.as_ref().is_some_and(|p| p.bool(name));
        let ignore_dyn = flag("bIgnoreDynLight");
        let no_ambient = flag("bNoAmbientLight");
        let pivot_ambient = flag("bUseOnlyPivotAmbient");
        let own_zone = flag("bVisibleOnlyOwnZone");
        let glow = a.as_ref().map_or(0, |p| p.byte("AmbientGlow"));
        let light_group = a.as_ref().map_or(0, |p| p.byte("bSLightGroup"));
        // Region: set by AActor::SetZone (UModel::PointRegion of Location) at load time.
        let region_zone = level.point_zone(actor.location);
        let draw_scale = a.as_ref().map_or(1.0, |p| p.float_or("DrawScale", 1.0));
        let scale3d = a.as_ref().and_then(|p| p.vec3("DrawScale3D")).unwrap_or([1.0; 3]);
        let l2w = local_to_world(actor.rotation, scale3d, draw_scale, actor.location, actor.pre_pivot);
        // AActor::UpdateLeaves: leaves touched by the transformed bounding box.
        let mut leaves = Vec::new();
        if let Some(model) = &level.model {
            let (bmin, bmax) = crate::lighting::static_mesh_world_box(mesh.bounds_min, mesh.bounds_max, &l2w);
            leaves = box_leaves(model, bmin, bmax);
            if crate::diag::static_mesh_actor().is_some_and(|n| n.eq_ignore_ascii_case(&actor.actor)) {
                eprintln!("{}: mesh box {:?} {:?}, world box {bmin:?} {bmax:?}", actor.actor, mesh.bounds_min, mesh.bounds_max);
            }
            if own_zone {
                leaves.retain(|&l| model.leaves.get(l as usize).copied() == Some(region_zone as i32));
            }
        }
        let leaf_zones: Vec<i32> = leaves.iter().filter_map(|&l| level.model.as_ref().and_then(|m| m.leaves.get(l as usize).copied())).collect();
        let leaf_lights: Vec<Vec<usize>> =
            leaves.iter().filter(|&&l| level.model.as_ref().is_some_and(|m| m.leaves.get(l as usize).is_some())).map(|&l| level.model.as_ref().unwrap().leaf_lights(l as usize)).collect();
        let zone_list: Vec<usize> = leaf_zones.iter().map(|&z| z.max(0) as usize).collect();
        let ambient_of = |z: usize| level.ambient(z);
        // FDynamicActor::Refresh: D3DRS_AMBIENT.
        let actor_ambient = if ignore_dyn {
            [0; 3]
        } else {
            let max =
                if !pivot_ambient && leaves.len() > 1 { zones_ambient_max(&zone_list, &ambient_of) } else { zones_ambient_max(&[region_zone], &ambient_of) };
            ambient_color(max, glow)
        };
        // Per-vertex colours: UStaticMeshInstance, recomputed by CalculateLighting on the first
        // render according to 's conditions.
        let instance = a.as_ref().and_then(|p| p.obj("StaticMeshInstance")).and_then(|path| {
            let (loaded, index) = library.resolve(&path)?;
            crate::static_mesh::MeshInstance::read(&loaded.package, &loaded.data, index).ok()
        });
        let colors: Option<Vec<[u8; 3]>> = instance.as_ref().map(|inst| {
            let lit = |idx: i16| level.lights.get(idx as usize).is_none_or(|l| l.dynamic || l.animated);
            let recalc = if ignore_dyn { true } else { inst.lights.iter().any(|i| i.light != -1 && lit(i.light) as i32 == i.applied) };
            if recalc || inst.colors.len() != mesh.positions.len() {
                let base = if ignore_dyn && !no_ambient {
                    let max = if !pivot_ambient { zones_ambient_max(&zone_list, &ambient_of) } else { zones_ambient_max(&[region_zone], &ambient_of) };
                    ambient_color(max, glow)
                } else {
                    [0; 3]
                };
                calculate_lighting(&mesh.positions, &mesh.normals, &l2w, base, &inst.lights, &level.lights)
            } else {
                inst.colors.iter().map(|c| [c[2], c[1], c[0]]).collect()
            }
        });
        if crate::diag::static_mesh_actor().is_some_and(|n| n.eq_ignore_ascii_case(&actor.actor)) {
            let avg = colors.as_ref().map(|c| {
                let n = c.len().max(1) as u32;
                let s = c.iter().fold([0u32; 3], |a, v| [a[0] + v[0] as u32, a[1] + v[1] as u32, a[2] + v[2] as u32]);
                [s[0] / n, s[1] / n, s[2] / n]
            });
            let saved = instance.as_ref().map(|i| {
                let n = i.colors.len().max(1) as u32;
                let s = i.colors.iter().fold([0u32; 3], |a, v| [a[0] + v[2] as u32, a[1] + v[1] as u32, a[2] + v[0] as u32]);
                (i.colors.len(), mesh.positions.len(), [s[0] / n, s[1] / n, s[2] / n])
            });
            if let Some(inst) = &instance {
                for info in &inst.lights {
                    let one = calculate_lighting(&mesh.positions, &mesh.normals, &l2w, [0; 3], std::slice::from_ref(info), &level.lights);
                    let n = one.len().max(1) as u32;
                    let s = one.iter().fold([0u32; 3], |a, v| [a[0] + v[0] as u32, a[1] + v[1] as u32, a[2] + v[2] as u32]);
                    let bits: u32 = info.visibility.iter().map(|b| b.count_ones()).sum();
                    let l = level.lights.get(info.light.max(0) as usize);
                    eprintln!(
                        "   light {}: effect {:?} color {:?} radius {:?} visible vertices {bits} → average {:?}",
                        info.light,
                        l.map(|l| l.effect),
                        l.map(|l| l.color),
                        l.map(|l| l.radius),
                        [s[0] / n, s[1] / n, s[2] / n]
                    );
                }
            }
            eprintln!(
                "{}: instance {} (saved {saved:?}) lights {:?} average colors {avg:?} ambient {actor_ambient:?} ignore_dyn {ignore_dyn} leaves {leaves:?} zones {leaf_zones:?} zone {region_zone}",
                actor.actor,
                instance.is_some(),
                instance.as_ref().map(|i| i.lights.iter().map(|l| (l.light, l.applied, level.lights.get(l.light as usize).map(|x| (x.dynamic, x.animated)))).collect::<Vec<_>>())
            );
        }
        // bUnlit: neither lighting nor per-vertex colours, white global colour.
        let unlit = flag("bUnlit");
        let state = if unlit {
            DrawState { lightmap: None, two_sided: false, lighting: false, color_vertex: false, has_color: false, ambient: [0; 3], local_normals: false, zone: Some(region_zone as u8) }
        } else {
            DrawState {
                lightmap: None,
                two_sided: false,
                lighting: !ignore_dyn,
                color_vertex: true,
                has_color: colors.is_some(),
                ambient: if no_ambient { [0; 3] } else { actor_ambient },
                local_normals: false,
                zone: Some(region_zone as u8),
            }
        };
        let per_actor = mesh.outline() || mesh.cartoon();
        let rotation = crate::level::rotation_matrix(actor.rotation);
        let base = b.world.vertices.len() as u32;
        for (i, p) in mesh.positions.iter().enumerate() {
            let n = mesh.normals[i];
            let normal =
                if per_actor {
                    n
                } else {
                    normalize([0, 1, 2].map(|k| {
                        (n[0] / actor.scale[0]) * rotation[0][k] + (n[1] / actor.scale[1]) * rotation[1][k] + (n[2] / actor.scale[2]) * rotation[2][k]
                    }))
                };
            let uv = mesh.uvs.first().and_then(|u| u.get(i)).copied().unwrap_or([0.0; 2]);
            let uv2 = mesh.uvs.get(1).and_then(|u| u.get(i)).copied().unwrap_or([0.0; 2]);
            let c = colors.as_ref().and_then(|c| c.get(i)).copied().unwrap_or([255; 3]);
            // The actor's LocalToWorld and FMatrix::TransformFVector, as the original
            // renderer and its projector triangles do (without SetZBias): bit-identical
            // depths, so the shadow no longer z-fights with the floor.
            b.vertex_full(Vertex { position: transform_point(&l2w, *p), normal, uv }, [c[0], c[1], c[2], 255], uv2);
        }
        let mirrored = actor.scale.iter().filter(|s| **s < 0.0).count() % 2 == 1;
        // SinusModifier materials: vertices of their section copied separately.
        let sine: Vec<Option<crate::sine::SineParams>> =
            materials.iter().map(|m| props.object(library, m).filter(|p| p.is_a("SinusModifier")).map(crate::sine::SineParams::read)).collect();
        let fade = if sine.iter().any(Option::is_some) {
            let per_material: Vec<(u8, Vec<[f32; 3]>)> = mesh
                .sections
                .iter()
                .enumerate()
                .filter_map(|(s, section)| {
                    let p = sine.get(s)?.as_ref()?;
                    Some((p.fade, mesh.section_triangles(section).iter().flatten().map(|&i| mesh.positions[i as usize]).collect()))
                })
                .collect();
            crate::sine::fade_bounds(&per_material, [0.0; 2])
        } else {
            [0.0; 2]
        };
        let mut sections = Vec::new();
        let mut section_indices = Vec::new();
        b.state = state.clone();
        // Each actor keeps its batches: the VM may hide it, destroy it or change its mesh.
        b.owner = Some(actor.actor.to_lowercase());
        for (s, section) in mesh.sections.iter().enumerate() {
            let material = materials.get(s).cloned().unwrap_or_default();
            let mut indices = Vec::new();
            let mut copy: HashMap<u16, u32> = HashMap::new();
            let mut local = Vec::new();
            let is_sine = sine.get(s).is_some_and(Option::is_some);
            let sine_base = b.world.vertices.len() as u32;
            for t in mesh.section_triangles(section) {
                let [x, y, z] = t.map(|i| base + i as u32);
                let [x, y, z] = if is_sine {
                    t.map(|i| {
                        *copy.entry(i).or_insert_with(|| {
                            let src = (base + i as u32) as usize;
                            let (v, c, uv2) = (b.world.vertices[src], b.world.colors[src], b.world.uv2[src]);
                            local.push(mesh.positions[i as usize]);
                            b.vertex_full(v, c, uv2)
                        })
                    })
                } else {
                    [x, y, z]
                };
                let tri = if mirrored { [x, z, y] } else { [x, y, z] };
                if per_actor {
                    indices.extend(tri);
                } else {
                    b.triangle(&material, tri[0], tri[1], tri[2]);
                }
                if actor.blocks && !is_outline_shell(&material) {
                    b.collide(x, y, z, &material);
                }
            }
            if is_sine {
                b.world.sine.push(SineSection {
                    modifier: material.clone(),
                    owner: Some(actor.actor.to_lowercase()),
                    base: sine_base,
                    local,
                    actor: actor.clone(),
                    fade,
                });
            }
            if per_actor {
                sections.push((material, 0, 0));
                section_indices.push(indices);
            }
        }
        b.state = DrawState::default();
        let owner = b.owner.take();
        if per_actor {
            let zone = level.zone(region_zone);
            let desc = crate::cartoon::CartoonDesc {
                outline: mesh.outline(),
                // bUnlit (Actor+0x30 & 0x20000): clears the cartoon flag read;
                // the outline stays.
                cartoon: mesh.cartoon() && !unlit,
                position_normals: mesh.position_normals(),
                outline_factor: mesh.outline_factor(),
                light_curve: zone.map_or(0, |z| z.byte("LightCurveShadow") as u32 | (z.byte("LightCurveBright") as u32) << 8),
                ambient: zone.map_or(0, |z| z.byte("AmbientIntensity")) as f32 * crate::lighting::INV_255,
                ..Default::default()
            };
            let max_scale = scale3d.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let center = transform_point(&l2w, [mesh.sphere[0], mesh.sphere[1], mesh.sphere[2]]);
            let radius = mesh.sphere[3] * draw_scale.abs() * max_scale;
            b.world.actor_meshes.push(ActorMesh {
                sections,
                desc,
                local_to_world: l2w,
                location: actor.location,
                scale: draw_scale.abs() * max_scale,
                sphere: [center[0], center[1], center[2], radius],
                light_group,
                leaves,
                // Zones read only by the light selection: gives no light to a
                // bIgnoreDynLight actor in the game.
                leaf_zones: if ignore_dyn { Vec::new() } else { leaf_zones },
                leaf_lights,
                state: DrawState { local_normals: true, ..state },
                mirrored,
                owner,
            });
            b.actor_indices.push(section_indices);
        }
    }
}

/// BSP as Engine.dll draws it (per surface): vertex fans from the
/// FBspVertexStream streams of each node, surface material (or LevelInfo.DefaultTexture),
/// editor lightmap (UModel+0xEC, if PolyFlags lacks bit 0x40), double-sided faces for
/// PF_TwoSided and portals. Invisible surfaces, portals, mirrors and sky backdrops
/// (PF_FakeBackdrop, drawn by the sky zone) are skipped as.
fn add_bsp(b: &mut Builder, map_name: &str, map: &Package, model: &Model, default_texture: &str, level: &LevelLighting) {
    // FLightManager::Cache, once per surface (the first zone drawn sets the ambient light).
    let mut cached: HashMap<usize, Option<String>> = HashMap::new();
    let mut surface_lights: Option<Vec<Vec<usize>>> = None;
    for (i_node, node) in model.nodes.iter().enumerate() {
        b.node = i_node as u32;
        if node.num_vertices < 3 || node.vertex_stream < 0 || node.vertex_index < 0 {
            continue;
        }
        let Some(stream) = model.vertex_streams.get(node.vertex_stream as usize) else { continue };
        let start = node.vertex_index as usize;
        let Some(poly) = stream.get(start..start + node.num_vertices as usize) else { continue };
        let surf = &model.surfs[node.surf as usize];
        let flags = surf.poly_flags;
        if flags & (PF_FAKE_BACKDROP | PF_MIRRORED | PF_PORTAL | PF_INVISIBLE) != 0 {
            continue;
        }
        let material = if surf.material == 0 { default_texture.to_string() } else { qualified(map_name, map, surf.material) };
        let lightmap = (flags & 0x40 == 0)
            .then(|| model.lightmap_textures.get(surf.lightmap_texture as usize))
            .flatten()
            .filter(|&&t| t != 0)
            .map(|&t| qualified(map_name, map, t))
            .or_else(|| {
                let index = node.surf as usize;
                cached
                    .entry(index)
                    .or_insert_with(|| {
                        // Zone of the node's front face (draw list).
                        let a = level.ambient(node.zone[1] as usize);
                        let dynamic = surface_lights.get_or_insert_with(|| crate::lightmap::surface_dynamic_lights(model, &level.lights));
                        let lm = crate::lightmap::cache(model, &level.lights, index, (a.hue, a.saturation, a.brightness), &dynamic[index])?;
                        let name = format!("#lightmap:{index}");
                        b.world.runtime_lightmaps.push((name.clone(), lm));
                        Some(name)
                    })
                    .clone()
            });
        // EnableLighting(0, PlateForm == 1, lightmap, …): PlateForm is 0 on PC (XIII.ini).
        // Fog of the node's front zone (zone of the draw list entry).
        b.state = DrawState { lightmap, two_sided: flags & 0x0400_0100 != 0, has_color: true, zone: Some(node.zone[1]), ..Default::default() };
        // FBspVertex (32 bytes: position, colour, two UVs) has no normal: D3DDrv's vertex
        // declaration does not provide one, D3D uses (0, 0, 0). Observed in the original
        // (window h2verrefumee_2sides_env of DM_LostTemple): 32-byte stream, stage 2
        // in D3DTSS_TCI_CAMERASPACEREFLECTIONVECTOR without transformation; the reflection therefore follows
        // the view direction, not the surface plane.
        let normal = [0.0; 3];
        let _ = surf.plane;
        let base = b.world.vertices.len() as u32;
        for v in poly {
            // FBspVertex: colour as B, G, R, A.
            let color = [v.color[2], v.color[1], v.color[0], v.color[3]];
            b.vertex_full(Vertex { position: v.position, normal, uv: v.uv }, color, v.uv2);
        }
        for k in 1..poly.len() as u32 - 1 {
            b.triangle(&material, base, base + k, base + k + 1);
        }
    }
    b.state = DrawState::default();
    b.node = u32::MAX;
    for (surf_index, poly) in model.collision_polygons_with_surfs() {
        let m = model.surfs[surf_index].material;
        let material = if m == 0 { String::new() } else { qualified(map_name, map, m) };
        for k in 1..poly.len() - 1 {
            b.collide_points([poly[0], poly[k], poly[k + 1]], &material);
        }
    }
}

/// Node of the terrain triangles in the Builder: TERRAIN | index in `World::terrain_names`.
const TERRAIN: u32 = 0x8000_0000;

fn add_terrain(b: &mut Builder, library: &mut Library, map_name: &str, map: &Package, data: &[u8], lighting: &LevelLighting) {
    // Not the terrains deleted in the editor (Hual01b: TerrainInfo0, bDeleteMe, under the snow).
    let level = crate::level::level_actor_exports(map, data);
    for i in 0..map.exports.len() {
        if map.export_class(i) != "TerrainInfo" || level.as_ref().is_some_and(|l| !l.contains(&i)) {
            continue;
        }
        let Ok(o) = map.object_data(data, i) else {
            continue;
        };
        // Fog of the TerrainInfo's zone (Region.ZoneNumber, actor drawn).
        let zone = match o.get(map, "Location") { Some(Value::Vector(l)) => lighting.point_zone(*l), _ => lighting.point_zone([0.0; 3]) } as u8;
        let Some(mut loaded) = crate::terrain_postload::read(library, map_name, map, data, i) else { continue };
        if loaded.post_load(library, |message| eprintln!("{message}")).is_err() { continue; }
        if loaded.terrain_map.is_none() { continue; }
        let t = &loaded.data;
        let (w, h) = (t.heightmap_x as usize, t.heightmap_y as usize);
        if t.vertices.len() < w * h || w < 2 || h < 2 { continue; }
        b.node = TERRAIN | b.world.terrain_names.len() as u32;
        b.world.terrain_names.push(map.object_name(openxiii_package::ObjectRef::Export(i)).to_lowercase());
        let visible = &loaded.visible;
        let turned = &loaded.turned;
        let layers = &loaded.layers;
        let maps = &loaded.maps;
        let alpha_texture = &loaded.alpha_texture;
        // Key prefixed with the layer index: batches are sorted by key and layers must be
        // drawn in order.
        let keys: Vec<Option<String>> = layers
            .iter()
            .enumerate()
            .map(|(k, l)| match (&l.texture, &l.alpha_map) {
                (Some(tex), Some(am)) => {
                    let m = crate::terrain::layer_matrix(&t, l);
                    let hex = m.iter().map(|f| format!("{:08x}", f.to_bits())).collect::<Vec<_>>().join(",");
                    Some(format!("#terrain:{k:02}:{hex}|{tex}|{am}"))
                }
                _ => None,
            })
            .collect();
        let vertex = |x: usize, y: usize| t.vertices[y * w + x];
        // Vertex normal (for materials that generate coordinates from it): differences
        // of the neighbours.
        let normal = |x: usize, y: usize| {
            let (xl, xr) = (x.saturating_sub(1), (x + 1).min(w - 1));
            let (yu, yd) = (y.saturating_sub(1), (y + 1).min(h - 1));
            let (a, c) = (vertex(xl, y), vertex(xr, y));
            let (bb, d) = (vertex(x, yu), vertex(x, yd));
            let dx = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let dy = [d[0] - bb[0], d[1] - bb[1], d[2] - bb[2]];
            normalize([dx[1] * dy[2] - dx[2] * dy[1], dx[2] * dy[0] - dx[0] * dy[2], dx[0] * dy[1] - dx[1] * dy[0]])
        };
        // UTerrainSector::UpdateVertexBuffer and ATerrainInfo::Render: sector vertices (position,
        // alpha map coordinates, baked colour), then each layer drawn with
        // EnableLighting(0, 1) and the material from UTerrainRenderMaterial::SetTerrain.
        for (slot, index) in loaded.sector_slots.iter().enumerate() {
            let Some(index) = index else { continue };
            let s = &t.sectors[*index];
            let sl = loaded.buffer_for_slot(slot).map(|b| b.layers.clone()).unwrap_or_else(|| crate::terrain::sector_layers(t, s, layers, maps, alpha_texture, visible, turned));
            if sl.is_empty() {
                continue;
            }
            let base = b.world.vertices.len() as u32;
            let mut k = 0usize;
            for y in 0..=s.quads_y {
                for x in 0..=s.quads_x {
                    let (gx, gy) = ((s.offset_x + x) as usize, (s.offset_y + y) as usize);
                    let gx = gx.min(w - 1);
                    let gy = gy.min(h - 1);
                    let c = s.colors.get(k).copied().unwrap_or([255; 4]);
                    let uv = [((s.offset_x + x) as f32 + 0.5) / t.heightmap_x as f32, ((s.offset_y + y) as f32 + 0.5) / t.heightmap_y as f32];
                    let generated = loaded.buffer_for_slot(slot).and_then(|b| b.vertices.get(k));
                    let (position, uv) = generated.map_or((vertex(gx, gy), uv), |v| (v.position, v.uv));
                    b.vertex_full(Vertex { position, normal: normal(gx, gy), uv }, [c[2], c[1], c[0], c[3]], [0.0; 2]);
                    k += 1;
                }
            }
            b.state = DrawState { has_color: true, color_vertex: true, zone: Some(zone), ..Default::default() };
            for layer in sl {
                let Some(key) = keys.get(layer.layer).cloned().flatten() else { continue };
                for tri in layer.indices.as_chunks::<3>().0 {
                    b.triangle(&key, base + tri[0], base + tri[1], base + tri[2]);
                }
            }
            b.state = DrawState::default();
        }
        b.node = u32::MAX;
        // Collision: visible quads, material of the visible layer (the highest above 50 %).
        let quad_materials = loaded.quad_materials();
        let bit = |b: &Vec<u32>, q: usize| b.get(q / 32).is_some_and(|v| v >> (q % 32) & 1 == 1);
        for y in 0..h - 1 {
            for x in 0..w - 1 {
                let q = y * w + x;
                if !bit(&visible, q) {
                    continue;
                }
                let material = quad_materials.get(q).and_then(|&k| layers.get(k as usize)).and_then(|l| l.texture.clone()).unwrap_or_default();
                let (a, bb, c, d) = (vertex(x, y), vertex(x + 1, y), vertex(x, y + 1), vertex(x + 1, y + 1));
                if bit(&turned, q) {
                    b.collide_points([a, d, bb], &material);
                    b.collide_points([a, c, d], &material);
                } else {
                    b.collide_points([a, c, bb], &material);
                    b.collide_points([bb, c, d], &material);
                }
            }
        }
    }
}

/// bAlphaTexture of a texture (UTexture::RequiresSorting).
fn props_alpha_texture(library: &mut Library, path: &str) -> bool {
    let Some((loaded, index)) = library.resolve(path) else { return false };
    let Ok(o) = loaded.package.object_data(&loaded.data, index) else { return false };
    matches!(o.get(&loaded.package, "bAlphaTexture"), Some(Value::Bool(true)))
}

/// Terrain layer: texture, alpha map (weights 0-255) and size of one repetition.
struct TerrainLayer {
    material: String,
    alpha: Option<(usize, usize, Vec<u8>)>,
}

impl TerrainLayer {
    fn weight(&self, u: f32, v: f32) -> u8 {
        match &self.alpha {
            Some((w, h, a)) => {
                let x = ((u * *w as f32) as usize).min(w - 1);
                let y = ((v * *h as f32) as usize).min(h - 1);
                a[y * w + x]
            }
            None => 255,
        }
    }
}

/// Layers `Layers[i]` of a TerrainInfo. XIII native format of each element: texture
/// (compact), alpha map (compact), UScale, VScale, UPan, VPan (f32), TextureMapAxis (u8),
/// TextureRotation (f32), LayerRotation (3 × i32).
fn terrain_layers(library: &mut Library, map_name: &str, map: &Package, o: &openxiii_package::ObjectData) -> Vec<TerrainLayer> {
    let mut layers = Vec::new();
    for p in o.properties.iter().filter(|p| map.name(p.name).eq_ignore_ascii_case("Layers")) {
        let Value::Raw(raw) = &p.value else { continue };
        let mut r = openxiii_package::Reader::new(raw, 0);
        let (Ok(texture), Ok(alpha)) = (r.compact(), r.compact()) else {
            continue;
        };
        if texture == 0 {
            continue;
        }
        let material = qualified(map_name, map, texture);
        let alpha = (alpha != 0).then(|| qualified(map_name, map, alpha)).and_then(|path| {
            let (loaded, index) = library.resolve(&path)?;
            let t = Texture::read(&loaded.package, &loaded.data, index).ok()?;
            let (w, h, rgba) = t.decode(0).ok()?;
            let channel = if t.format == Format::L8 || t.format == Format::P8 { 0 } else { 3 };
            Some((w as usize, h as usize, rgba.chunks_exact(4).map(|px| px[channel]).collect()))
        });
        layers.push((p.array_index, TerrainLayer { material, alpha }));
    }
    layers.sort_by_key(|(i, _)| *i);
    layers.into_iter().map(|(_, l)| l).collect()
}

/// The map's zones: nodes for PointRegion, fog and sky of each zone.
fn add_zones(world: &mut World, library: &mut Library, map_name: &str, map: &Package, data: &[u8], level: &LevelLighting, props: &mut crate::props::PropCache) {
    let Some(model) = &level.model else { return };
    world.region_nodes = level.nodes.clone();
    world.root_outside = model.root_outside;
    let world_nodes = world.region_nodes.clone();
    // ZoneInfo.LinkToSkybox (PreBeginPlay): SkyZoneUser, otherwise the last SkyZoneInfo of the
    // actor list, preferably the one whose bHighDetail equals Level.bHighDetailMode.
    // AllActors: only the actors of ULevel::Actors, in that array's order (Hual01b:
    // SkyZoneInfo1, deleted in the editor, is the last export of the class; USA01:
    // SkyZoneInfo0, outside the world, comes after SkyZoneInfo1 among the exports but before it in
    // Actors; otherwise the sky stays black).
    let mut skies: Vec<(String, crate::props::Props)> = Vec::new();
    let order: Vec<usize> = crate::level::level_actor_order(map, data).unwrap_or_else(|| (0..map.exports.len()).collect());
    for i in order {
        if map.export_class(i) == "SkyZoneInfo" {
            let path = qualified(map_name, map, i as i32 + 1);
            if let Some(p) = props.object(library, &path).cloned() {
                skies.push((path, p));
            }
        }
    }
    let high_detail = level.level_info.as_ref().is_some_and(|l| l.bool("bHighDetailMode"));
    let default_sky = skies.iter().rev().find(|(_, p)| p.bool("bHighDetail") == high_detail).or(skies.last()).map(|(path, _)| path.clone());
    let sky_info = |path: &str, props: &mut crate::props::PropCache, library: &mut Library| {
        let p = props.object(library, path)?;
        // Region of the SkyZoneInfo: set at load time by AActor::SetZone (PointRegion).
        let location = p.vec3("Location").unwrap_or([0.0; 3]);
        let zone = point_zone(&world_nodes, model.root_outside, location) as u8;
        // FSkySceneNode (with SkyZone.Region.ZoneNumber) places the
        // camera at the Location and Rotation of that zone's actor (Model.Zones[n].ZoneActor), not of the chosen SkyZoneInfo: DM_LostTemple has two SkyZoneInfo
        // in the same zone, LinkToSkybox picks SkyZoneInfo0 and the zone has SkyZoneInfo2.
        let zone_actor = level.zones.get(zone as usize).and_then(Option::as_ref);
        let (location, rotation) = match zone_actor {
            Some(z) => (z.vec3("Location").unwrap_or([0.0; 3]), z.rot("Rotation")),
            None => (location, p.rot("Rotation")),
        };
        Some((location, rotation, zone))
    };
    let count = model.zones.len().max(1);
    for z in 0..count {
        let zone = level.zone(z).cloned();
        let fog = zone.as_ref().filter(|p| p.bool("bDistanceFog")).map(|p| {
            // Color struct serialized in field order (B, G, R, A, Object.uc).
            let c = p.raw("DistanceFogColor").unwrap_or(&[0, 0, 0, 0]).to_vec();
            (p.float("DistanceFogStart"), p.float("DistanceFogEnd"), [c[2], c[1], c[0]])
        });
        let clear = zone.as_ref().filter(|p| p.bool("bForceClear")).map(|p| {
            let c = p.raw("DistanceFogColor").unwrap_or(&[0, 0, 0, 0]).to_vec();
            [c[2], c[1], c[0]]
        });
        let sky_path = zone.as_ref().and_then(|p| p.obj("SkyZoneUser")).or_else(|| default_sky.clone());
        let sky = if zone.as_ref().is_some_and(|p| p.is_a("ZoneInfo")) { sky_path.and_then(|path| sky_info(&path, props, library)) } else { None };
        let light_curve = zone.as_ref().map_or(0, |z| z.byte("LightCurveShadow") as u32 | (z.byte("LightCurveBright") as u32) << 8);
        let ambient_intensity = zone.as_ref().map_or(0, |z| z.byte("AmbientIntensity")) as f32 * crate::lighting::INV_255;
        let a = level.ambient(z);
        let ambient_hsv = crate::lighting::zones_ambient_max(&[z], &|_| a);
        let ambient = crate::lighting::ambient_color(ambient_hsv, 0);
        let fog_per_zone = zone.as_ref().is_some_and(|p| p.bool("bFogPerZone"));
        world.zones.push(ZoneRender { fog, clear, sky, light_curve, ambient_intensity, ambient, ambient_hsv, fog_per_zone });
    }
}

/// LevelInfo.DefaultTexture (material of BSP surfaces that have none).
fn level_default_texture(map_name: &str, map: &Package, data: &[u8]) -> String {
    for i in 0..map.exports.len() {
        if map.export_class(i) == "LevelInfo" {
            if let Ok(o) = map.object_data(data, i) {
                if let Some(Value::Object(t)) = o.get(map, "DefaultTexture") {
                    if *t != 0 {
                        return qualified(map_name, map, *t);
                    }
                }
            }
            break;
        }
    }
    // Default value of the LevelInfo class.
    "Engine.DefaultTexture".to_string()
}

/// Builds the complete geometry of a map already inserted into the library.
pub fn build_world(library: &mut Library, map_name: &str) -> Option<World> {
    build_world_excluding(library, map_name, &std::collections::HashSet::new())
}

/// Like `build_world`, without the actors named in `exclude` (e.g. Movers animated separately).
pub fn build_world_excluding(library: &mut Library, map_name: &str, exclude: &std::collections::HashSet<String>) -> Option<World> {
    let (map, data) = {
        let loaded = library.get(map_name)?;
        (loaded.package.clone(), loaded.data.clone())
    };
    let mut b = Builder { world: World::default(), actor_indices: Vec::new(), groups: HashMap::new(), state: DrawState::default(), owner: None, node: u32::MAX, group_nodes: HashMap::new() };
    let actors: Vec<MeshActor> = mesh_actors(map_name, &map, &data).into_iter().filter(|a| !exclude.contains(&a.actor.to_lowercase())).collect();
    let mut props = crate::props::PropCache::new();
    let level = LevelLighting::new(library, &mut props, map_name, &map, &data);
    add_static_meshes(&mut b, library, map_name, &actors, &level, &mut props);
    b.world.lights = level.lights.clone();
    add_zones(&mut b.world, library, map_name, &map, &data, &level, &mut props);
    let default_texture = level_default_texture(map_name, &map, &data);
    if let Some(index) = level_model(&map, &data) {
        if let Ok(model) = Model::read(&map, &data, index) {
            add_bsp(&mut b, map_name, &map, &model, &default_texture, &level);
            b.world.light_records = model.lights.clone();
            b.world.model = Some(std::sync::Arc::new(model));
        }
    }
    b.world.level_brightness = level.brightness;
    add_terrain(&mut b, library, map_name, &map, &data, &level);
    let in_level = crate::level::level_actor_exports(&map, &data);
    for i in 0..map.exports.len() {
        if map.export_class(i) == "PlayerStart" && in_level.as_ref().is_none_or(|l| l.contains(&i)) {
            if let Ok(o) = map.object_data(&data, i) {
                if let Some(Value::Vector(l)) = o.get(&map, "Location") {
                    let yaw = match o.get(&map, "Rotation") {
                        Some(Value::Rotator(r)) => r[1],
                        _ => 0,
                    };
                    b.world.player_start = Some((*l, yaw));
                    break;
                }
            }
        }
    }
    let mut w = b.finish();
    w.map_value = map_name.to_string();
    Some(w)
}

/// StaticMeshes of map actors whose script changed the mesh (Amos01: DisplayScreen),
/// built and lit as in `build_world` (the actor's UStaticMeshInstance):
/// (actor name, new StaticMesh).
pub fn build_mesh_replacements(library: &mut Library, map_name: &str, replacements: &[(String, String)]) -> Option<World> {
    build_mesh_replacements_kept(library, map_name, replacements, &mut None)
}

/// What `build_mesh_replacements` reads from the map on each call and that does not depend on it:
/// the map's placed StaticMeshes (`mesh_actors`), level lighting (`LevelLighting::new`,
/// BSP included, lights in their UModel::PostLoad state) and the Props read. Everything comes from the
/// map package's data, which does not change during the level.
pub struct BaseReplacements {
    map_name: String,
    actors: Vec<MeshActor>,
    level: LevelLighting,
    props: crate::props::PropCache,
}

/// `build_mesh_replacements` with `base` kept from one call to the next (rebuilt for another
/// map): same scenery, without re-reading the map's actors or BSP for each broken shutter.
pub fn build_mesh_replacements_kept(library: &mut Library, map_name: &str, replacements: &[(String, String)], base: &mut Option<BaseReplacements>) -> Option<World> {
    if base.as_ref().is_none_or(|b| b.map_name != map_name) {
        let (map, data) = {
            let loaded = library.get(map_name)?;
            (loaded.package.clone(), loaded.data.clone())
        };
        let actors = mesh_actors(map_name, &map, &data);
        let mut props = crate::props::PropCache::new();
        let level = LevelLighting::new(library, &mut props, map_name, &map, &data);
        *base = Some(BaseReplacements { map_name: map_name.to_owned(), actors, level, props });
    }
    let base = base.as_mut().unwrap();
    let mut b = Builder {
        world: World::default(),
        actor_indices: Vec::new(),
        groups: HashMap::new(),
        state: DrawState::default(),
        owner: None,
        node: u32::MAX,
        group_nodes: HashMap::new(),
    };
    let actors: Vec<MeshActor> = base
        .actors
        .iter()
        .filter_map(|a| {
            let (_, mesh) = replacements.iter().find(|(name, _)| name.eq_ignore_ascii_case(&a.actor))?;
            let mut a = a.clone();
            a.mesh = mesh.clone();
            Some(a)
        })
        .collect();
    add_static_meshes(&mut b, library, map_name, &actors, &base.level, &mut base.props);
    Some(b.finish())
}

/// Finds the displayable images of a material (all decodable mip levels):
/// the texture itself, or by following the usual properties of Shader/Combiner/Modifier
/// (Diffuse, Material, Material1…). Textures without a useful alpha channel are made opaque.
pub fn material_image(library: &mut Library, path: &str) -> Option<Vec<Image>> {
    let mut current = path.to_string();
    for _ in 0..6 {
        // Mods build only: a mod's texture for this name (mod_textures.rs).
        #[cfg(feature = "mods")]
        if let Some(m) = crate::mod_textures::get(&current) {
            return Some(m.iter().map(|(width, height, rgba)| Image { width: *width, height: *height, rgba: rgba.clone() }).collect());
        }
        let (loaded, index) = library.resolve(&current)?;
        let class = loaded.package.export_class(index).to_string();
        let o = loaded.package.object_data(&loaded.data, index).ok()?;
        if class == "Texture" {
            let flag = |name| matches!(o.get(&loaded.package, name), Some(Value::Bool(true)));
            let alpha = flag("bMasked") || flag("bAlphaTexture");
            let texture = Texture::read(&loaded.package, &loaded.data, index).ok()?;
            let mut images = Vec::new();
            for level in 0..texture.mips.len() {
                let Ok((width, height, mut rgba)) = texture.decode(level) else {
                    break;
                };
                if let Some(previous) = images.last() {
                    let previous: &Image = previous;
                    if width != (previous.width / 2).max(1) || height != (previous.height / 2).max(1) {
                        break;
                    }
                }
                if !alpha {
                    rgba.chunks_exact_mut(4).for_each(|px| px[3] = 255);
                }
                images.push(Image { width, height, rgba });
            }
            return (!images.is_empty()).then_some(images);
        }
        let next =
            ["Diffuse", "Material", "Material1", "Material2", "Texture", "SelfIllumination"].iter().find_map(|name| match o.get(&loaded.package, name) {
                Some(Value::Object(r)) if *r != 0 => Some(qualified(&loaded.name, &loaded.package, *r)),
                _ => None,
            })?;
        current = next;
    }
    None
}

/// Adds the reference pose of a skeletal mesh placed in the world
/// (characters and animated objects). Returns false if the mesh cannot be found.
pub fn add_skeletal(
    world: &mut World,
    library: &mut Library,
    mesh_path: &str,
    location: [f32; 3],
    rotation: [i32; 3],
    scale: [f32; 3],
    pre_pivot: [f32; 3],
    pose: Option<(&str, f32)>,
) -> bool {
    let Some((loaded, index)) = library.resolve(mesh_path) else {
        return false;
    };
    if loaded.package.export_class(index) != "SkeletalMesh" {
        return false;
    }
    let Ok(mesh) = crate::skeletal::SkeletalMesh::read(&loaded.package, &loaded.data, index) else {
        return false;
    };
    let mesh = match pose {
        Some((sequence, time)) => posed(library, mesh_path, &mesh, sequence, time).unwrap_or(mesh),
        None => mesh,
    };
    let Some((loaded, _)) = library.resolve(mesh_path) else {
        return false;
    };
    let actor = MeshActor { actor: String::new(), mesh: mesh_path.to_string(), location, rotation, scale, pre_pivot, blocks: false };
    let names: Vec<String> = (0..mesh.faces.len())
        .map(|f| {
            let t = mesh.face_texture(mesh.faces[f].material);
            if t == 0 {
                String::new()
            } else {
                qualified(&loaded.name, &loaded.package, t)
            }
        })
        .collect();
    let base = world.vertices.len() as u32;
    // One vertex per wedge (own UVs), normals recomputed from the faces.
    let positions: Vec<[f32; 3]> = (0..mesh.points.len()).map(|i| actor.transform(mesh.transformed_point(i))).collect();
    let mut normals = vec![[0.0f32; 3]; mesh.points.len()];
    for f in &mesh.faces {
        let [a, b, c] = f.wedges.map(|w| mesh.wedges[w as usize].point as usize);
        let (pa, pb, pc) = (positions[a], positions[b], positions[c]);
        let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        for i in [a, b, c] {
            for k in 0..3 {
                normals[i][k] += n[k];
            }
        }
    }
    for w in &mesh.wedges {
        let p = w.point as usize;
        world.vertices.push(Vertex { position: positions[p], normal: normalize(normals[p]), uv: w.uv });
        world.colors.push([255; 4]);
        world.uv2.push([0.0; 2]);
    }
    let mut groups: HashMap<String, Vec<u32>> = HashMap::new();
    for (i, f) in mesh.faces.iter().enumerate() {
        groups.entry(names[i].clone()).or_default().extend(f.wedges.map(|w| base + w as u32));
    }
    for (material, indices) in groups {
        let first_index = world.indices.len() as u32;
        world.batches.push(Batch { material, first_index, index_count: indices.len() as u32 });
        world.bsp_node_ranges.push(Vec::new());
        world.states.push(DrawState::default());
        world.indices.extend(indices);
    }
    true
}

/// Mesh deformed by sequence `sequence` of its default animation, at time `time` (s).
fn posed(library: &mut Library, mesh_path: &str, mesh: &crate::skeletal::SkeletalMesh, sequence: &str, time: f32) -> Option<crate::skeletal::SkeletalMesh> {
    let (mesh_pkg, anim_path, bone_names) = {
        let (loaded, _) = library.resolve(mesh_path)?;
        let anim_path = match ObjectRef::from(mesh.animation) {
            ObjectRef::None => return None,
            ObjectRef::Export(_) => format!("{}.{}", loaded.name, loaded.package.object_path(mesh.animation.into())),
            _ => loaded.package.object_path(mesh.animation.into()),
        };
        let names: HashMap<i32, String> = mesh.bones.iter().map(|b| (b.name, loaded.package.name(b.name).to_string())).collect();
        (loaded.name.clone(), anim_path, names)
    };
    let _ = mesh_pkg;
    let (loaded, index) = library.resolve(&anim_path)?;
    let anim = crate::animation::MeshAnimation::read(&loaded.package, &loaded.data, index).ok()?;
    let seq = anim.sequences.iter().position(|s| loaded.package.name(s.name).eq_ignore_ascii_case(sequence))?;
    let info = &anim.sequences[seq];
    let frames = info.num_frames.max(1) as f32;
    let frame = (time * info.rate).rem_euclid(frames);
    let pkg = &loaded.package;
    let points = mesh.pose(&anim, &|n| pkg.name(n).to_string(), &|n| bone_names.get(&n).cloned().unwrap_or_default(), seq, frame);
    Some(mesh.with_points(points))
}

/// Triangles of a StaticMesh in its own space (for collision of moving objects).
pub fn static_mesh_triangles(library: &mut Library, mesh_path: &str) -> Option<Vec<[[f32; 3]; 3]>> {
    let (loaded, index) = library.resolve(mesh_path)?;
    let mesh = StaticMesh::read(&loaded.package, &loaded.data, index).ok()?;
    let mut out = Vec::new();
    for section in &mesh.sections {
        for t in mesh.section_triangles(section) {
            out.push(t.map(|i| mesh.positions[i as usize]));
        }
    }
    Some(out)
}

/// A mod model's outline shell section (drawn by the outline pass only, never solid).
fn is_outline_shell(_material: &str) -> bool {
    #[cfg(feature = "mods")]
    if crate::mod_meshes::outline_shell_of(_material).is_some() {
        return true;
    }
    false
}
