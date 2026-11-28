//! ATerrainInfo::CalcCoords: writes the two frames of TerrainData.
//! CalcVertices and terrain collision read these same fields.
use crate::coords::Coords;
use crate::terrain::TerrainData;

pub fn calc_coords(data: &mut TerrainData, location: [f32; 3], terrain_scale: [f32; 3], has_terrain_map: bool) {
    // Each x87 step rounded to 24-bit precision, then a 48-byte copy.
    // Y first computes -1/scale, then multiplies Location.Y; not -Location.Y/scale.
    let mut world = Coords {
        origin: [-(location[0] / terrain_scale[0]), (-1.0_f32 / terrain_scale[1]) * location[1], (location[2] / terrain_scale[2]) * -256.0_f32],
        x: [terrain_scale[0], 0.0, 0.0],
        y: [0.0, terrain_scale[1], 0.0],
        z: [0.0, 0.0, terrain_scale[2] * 0.00390625_f32],
    };
    // Dimensions /2 truncated towards 0, G16 zero offset at 32767.
    if has_terrain_map {
        world.origin[0] += (data.heightmap_x / 2) as f32;
        world.origin[1] += (data.heightmap_y / 2) as f32;
        world.origin[2] += 32767.0_f32;
    }
    data.to_world = world;
    // Helper already reimplemented, its general contract unchanged.
    data.to_heightmap = world.pivot_inverse();
}
