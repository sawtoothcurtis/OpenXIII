//! SectorInit, on the Terrain loader's Sector type.
//!
//! The terrain reference is the parent's identity in the Rust package, in
//! place of the native UObject pointer. The bounds belong to the allocation and
//! stay untouched: the constructor does not compute them.
use crate::terrain::Sector;

/// Initialises the sector fields used by the Terrain consumers.
/// The caller provides the allocated sector (and so its existing bounds).
/// The arrays are freshly empty, colour revision zero; no
/// geometry, lighting or UpdateVertexBuffer replacement here.
pub fn initialize_sector(
    sector: &mut Sector,
    terrain_reference: i32,
    offset_x: i32,
    offset_y: i32,
    quads_x: i32,
    quads_y: i32,
) -> &mut Sector {
    sector.terrain_reference = terrain_reference;
    sector.quads_x = quads_x;
    sector.quads_y = quads_y;
    sector.offset_x = offset_x;
    sector.offset_y = offset_y;
    sector.colors = Vec::new();
    sector.lights = Vec::new();
    sector.color_revision = 0;
    sector
}
