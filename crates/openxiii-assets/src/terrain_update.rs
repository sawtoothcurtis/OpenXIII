//! Geometry steps of ATerrainInfo::Update.
//! The parent owns the loading/repair policy; these operations mutate its TerrainData.

use crate::coords;
use crate::terrain::{self, AlphaMap, Layer, SectorLayer, TerrainData};

/// Raw mip0 of TerrainMap. `width` is UTexture::USize (+54), not decoded RGBA width.
#[derive(Clone, Copy)]
pub struct Heightmap<'a> {
    pub format: u8,
    pub width: i32,
    pub pixels: &'a [u8],
}

/// Original half-open update rectangle. No clamping or sector normalization.
#[derive(Debug, Clone, Copy)]
pub struct Rectangle {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// CalcVertices.
///
/// `unused` preserves arg1 of the original five-argument call. `load_alpha` is the
/// lazy mip-load boundary: callers load the actual alpha bytes at this point.
/// A missing TerrainMap preserves all state and does not invoke that boundary.
/// New slots outside the written rectangle have unspecified native contents;
/// Rust initializes those slots to zero, while all existing slots are preserved.
pub fn calc_vertices(
    terrain: &mut TerrainData,
    heightmap: Option<Heightmap<'_>>,
    _unused: u32,
    rect: Rectangle,
    layers: &[Layer],
    mut load_alpha: impl FnMut(usize),
) {
    let Some(heightmap) = heightmap else { return };
    let needed = terrain.heightmap_x.wrapping_mul(terrain.heightmap_y).max(0) as usize;
    if terrain.vertices.len() < needed {
        terrain.vertices.resize(needed, [0.0; 3]);
    }
    for y in rect.min_y..rect.max_y {
        for x in rect.min_x..rect.max_x {
            let pixel = heightmap.width.wrapping_mul(y).wrapping_add(x) as usize;
            let height = match heightmap.format {
                0 | 9 => u16::from(heightmap.pixels[pixel]) << 8,
                10 => u16::from_le_bytes([heightmap.pixels[2 * pixel], heightmap.pixels[2 * pixel + 1]]),
                _ => 0,
            };
            let point = coords::transform_point([x as f32, y as f32, height as f32], &terrain.to_world);
            let index = terrain.heightmap_x.wrapping_mul(y).wrapping_add(x) as usize;
            terrain.vertices[index] = point;
        }
    }
    for (index, layer) in layers.iter().take(32).enumerate() {
        if layer.texture.is_none() || layer.alpha_map.is_none() {
            break;
        }
        load_alpha(index);
    }
}

/// Four original UTerrainSector fields read by UpdateVertexBuffers.
#[derive(Debug, Clone, Copy)]
pub struct SectorRectangle {
    pub quads_x: i32,
    pub quads_y: i32,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// Callee boundary for the wrapper. The production adapter below writes
/// the sector layers consumed by rendering; this trait also preserves re-reading
/// the sector list after each call rather than caching its count or references.
pub trait SectorBufferBackend {
    fn sector_count(&self) -> usize;
    fn sector_rectangle(&self, index: usize) -> SectorRectangle;
    fn update_sector_buffer(&mut self, index: usize);
    /// Separate boundary, not a reimplementation of StaticLight. PostLoad passes flag 0.
    fn static_light(&mut self, index: usize, force: u32);
}

/// UpdateVertexBuffers: four inclusive signed comparisons, wrapped i32
/// endpoints, then UpdateVertexBuffer and optional StaticLight(sector, 1).
pub fn update_vertex_buffers(backend: &mut impl SectorBufferBackend, rect: Rectangle, lighting: u32) {
    let mut index = 0;
    while index < backend.sector_count() {
        let sector = backend.sector_rectangle(index);
        if sector.offset_x <= rect.max_x
            && sector.offset_y <= rect.max_y
            && sector.offset_x.wrapping_add(sector.quads_x) >= rect.min_x
            && sector.offset_y.wrapping_add(sector.quads_y) >= rect.min_y
        {
            backend.update_sector_buffer(index);
            if lighting != 0 {
                backend.static_light(index, 1);
            }
        }
        index += 1;
    }
}

/// Inputs of the existing sector_layers producer.
pub struct BufferInputs<'a> {
    pub layers: &'a [Layer],
    pub maps: &'a [Option<AlphaMap>],
    pub alpha_texture: &'a [bool],
    pub visibility: &'a [u32],
    pub edge_turn: &'a [u32],
}

/// Geometry storage. Some(empty) is a rebuilt empty sector; None is an
/// untouched sector. The caller keeps these outputs and renders their indices.
/// StaticLight is supplied explicitly as an external boundary. Supplying a
/// handler does not define its semantics; the PostLoad caller uses 0.
pub struct SectorGeometryBackend<'a, F> {
    pub terrain: &'a mut TerrainData,
    pub inputs: BufferInputs<'a>,
    pub buffers: &'a mut Vec<Option<Vec<SectorLayer>>>,
    pub static_light_boundary: F,
}

impl<F: FnMut(&mut TerrainData, usize, u32)> SectorBufferBackend for SectorGeometryBackend<'_, F> {
    fn sector_count(&self) -> usize { self.terrain.sectors.len() }
    fn sector_rectangle(&self, index: usize) -> SectorRectangle {
        let sector = &self.terrain.sectors[index];
        SectorRectangle { quads_x: sector.quads_x, quads_y: sector.quads_y, offset_x: sector.offset_x, offset_y: sector.offset_y }
    }
    fn update_sector_buffer(&mut self, index: usize) {
        let layers = terrain::sector_layers(self.terrain, &self.terrain.sectors[index], self.inputs.layers,
            self.inputs.maps, self.inputs.alpha_texture, self.inputs.visibility, self.inputs.edge_turn);
        if self.buffers.len() <= index {
            self.buffers.resize_with(index + 1, || None);
        }
        self.buffers[index] = Some(layers);
    }
    fn static_light(&mut self, index: usize, force: u32) {
        (self.static_light_boundary)(self.terrain, index, force);
    }
}

/// Three original callees of Update, operating on the same terrain storage.
pub trait TerrainUpdateBackend {
    fn heightmap_x(&self) -> i32;
    fn heightmap_y(&self) -> i32;
    fn calc_vertices(&mut self, arguments: [u32; 5]);
    fn update_vertex_buffers(&mut self, arguments: [u32; 5]);
    fn compute_quad_materials(&mut self);
}

/// Update. Both defaulted endpoints are captured before the first callee.
pub fn update(backend: &mut impl TerrainUpdateBackend, arguments: [u32; 6]) {
    let end_x = if arguments[3] == 0 { backend.heightmap_x() as u32 } else { arguments[3] };
    let end_y = if arguments[4] == 0 { backend.heightmap_y() as u32 } else { arguments[4] };
    backend.calc_vertices([arguments[0], arguments[1], arguments[2], end_x, end_y]);
    backend.update_vertex_buffers([arguments[1], arguments[2], end_x, end_y, arguments[5]]);
    backend.compute_quad_materials();
}

/// Production adapter for the three callees. It writes the original terrain's
/// positions, retained sector layers and quad-material table. The parent repair
/// path passes those exact stores to rendering and collision.
///
/// `load_alpha` loads the actual mip bytes into `maps`. It is invoked for every
/// present prefix layer, including layers whose mip was already resident.
/// `static_light_boundary` remains the external; PostLoad invokes this backend
/// with argument 6 = 0, so that not reimplemented boundary is not called on its path.
pub struct TerrainGeometryBackend<'a, A, S> {
    pub terrain: &'a mut TerrainData,
    pub heightmap: Option<Heightmap<'a>>,
    pub layers: &'a [Layer],
    pub maps: &'a mut Vec<Option<AlphaMap>>,
    pub load_alpha: A,
    pub alpha_texture: &'a [bool],
    pub visibility: &'a [u32],
    pub edge_turn: &'a [u32],
    pub buffers: &'a mut Vec<Option<Vec<SectorLayer>>>,
    pub quad_materials: &'a mut Vec<u8>,
    pub static_light_boundary: S,
}

impl<A, S> TerrainUpdateBackend for TerrainGeometryBackend<'_, A, S>
where A: FnMut(usize) -> Option<AlphaMap>, S: FnMut(&mut TerrainData, usize, u32) {
    fn heightmap_x(&self) -> i32 { self.terrain.heightmap_x }
    fn heightmap_y(&self) -> i32 { self.terrain.heightmap_y }
    fn calc_vertices(&mut self, args: [u32; 5]) {
        let maps = &mut self.maps;
        let loader = &mut self.load_alpha;
        calc_vertices(self.terrain, self.heightmap, args[0], Rectangle {
            min_x: args[1] as i32, min_y: args[2] as i32, max_x: args[3] as i32, max_y: args[4] as i32,
        }, self.layers, |index| {
            if maps.len() <= index { maps.resize_with(index + 1, || None); }
            maps[index] = loader(index);
        });
    }
    fn update_vertex_buffers(&mut self, args: [u32; 5]) {
        let mut backend = SectorGeometryBackend { terrain: self.terrain,
            inputs: BufferInputs { layers: self.layers, maps: self.maps, alpha_texture: self.alpha_texture,
                visibility: self.visibility, edge_turn: self.edge_turn },
            buffers: self.buffers, static_light_boundary: &mut self.static_light_boundary };
        update_vertex_buffers(&mut backend, Rectangle { min_x: args[0] as i32, min_y: args[1] as i32,
            max_x: args[2] as i32, max_y: args[3] as i32 }, args[4]);
    }
    fn compute_quad_materials(&mut self) {
        *self.quad_materials = self.terrain.quad_materials(self.maps);
    }
}
