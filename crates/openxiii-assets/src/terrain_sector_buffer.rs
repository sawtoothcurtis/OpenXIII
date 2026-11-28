//! UTerrainSector::UpdateVertexBuffer.
//! The result is the vertex stream and layers directly consumed by World.
use crate::terrain::{self, AlphaMap, Layer, Sector, SectorLayer, TerrainData};
use crate::terrain_update::Heightmap;

#[derive(Debug, Clone)]
pub struct TerrainVertex { pub position:[f32;3], pub uv:[f32;2] }
#[derive(Debug, Clone, Default)]
pub struct SectorBuffer {
    pub vertices:Vec<TerrainVertex>,
    pub revision:u32,
    pub layers:Vec<SectorLayer>,
    pub sphere_center:[f32;3],
    pub sphere_radius:f32,
}

pub struct Inputs<'a>{
    pub heightmap:Option<Heightmap<'a>>,
    pub layers:&'a [Layer],
    pub maps:&'a [Option<AlphaMap>],
    pub alpha_texture:&'a [bool],
    pub visible:&'a [u32],
    pub turned:&'a [u32],
}

/// A missing TerrainMap returns before changing any sector or buffer state.
/// Editor-only wireframe indices are outside the runtime path (GIsEditor=0).
pub fn update_sector_buffer(data:&TerrainData,sector:&mut Sector,buffer:&mut SectorBuffer,inputs:Inputs<'_>){
    let Some(heightmap)=inputs.heightmap else{return};
    buffer.layers=terrain::sector_layers(data,sector,inputs.layers,inputs.maps,inputs.alpha_texture,inputs.visible,inputs.turned);
    let count=sector.quads_x.wrapping_add(1).wrapping_mul(sector.quads_y.wrapping_add(1)).max(0)as usize;
    let reset_colors=sector.colors.len()!=count;
    if reset_colors{sector.colors=vec![[255;4];count]}
    buffer.vertices.clear();buffer.vertices.reserve(count);
    let(mut low,mut high)=(i32::MAX,0i32);
    for y in 0..=sector.quads_y{
        for x in 0..=sector.quads_x{
            let gx=sector.offset_x.wrapping_add(x);let gy=sector.offset_y.wrapping_add(y);
            let pixel=heightmap.width.wrapping_mul(gy).wrapping_add(gx)as usize;
            let height=match heightmap.format{0|9=>u16::from(heightmap.pixels[pixel])<<8,10=>u16::from_le_bytes([heightmap.pixels[2*pixel],heightmap.pixels[2*pixel+1]]),_=>0};
            low=low.min(i32::from(height));high=high.max(i32::from(height));
            let index=data.heightmap_x.wrapping_mul(gy).wrapping_add(gx)as usize;
            buffer.vertices.push(TerrainVertex{position:data.vertices[index],uv:[(gx as f32+0.5)/data.heightmap_x as f32,(gy as f32+0.5)/data.heightmap_y as f32]});
        }
    }
    buffer.revision=buffer.revision.wrapping_add(1);
    if reset_colors{sector.color_revision=sector.color_revision.wrapping_add(1)}
    // Eight combinations, rows of ToWorld, f32 at each operation.
    let transform=|p:[f32;3]|{
        let c=&data.to_world;let d=[p[0]-c.origin[0],p[1]-c.origin[1],p[2]-c.origin[2]];
        let axes=[c.x,c.y,c.z]; std::array::from_fn(|k|(d[0]*axes[k][0]+d[1]*axes[k][1])+d[2]*axes[k][2])
    };
    for (i,p) in sector.bounds.iter_mut().enumerate(){
        let x=sector.offset_x.wrapping_add(if i&1!=0{sector.quads_x}else{0});
        let y=sector.offset_y.wrapping_add(if i&2!=0{sector.quads_y}else{0});
        *p=transform([x as f32,y as f32,if i&4!=0{high}else{low}as f32]);
    }
    // Centre from three actual global vertices, then radius from corner 0.
    let at=|x:i32,y:i32|data.vertices[data.heightmap_x.wrapping_mul(y).wrapping_add(x)as usize];
    let p=at(sector.offset_x,sector.offset_y);
    let x=at(sector.offset_x.wrapping_add(sector.quads_x),sector.offset_y);
    let y=at(sector.offset_x,sector.offset_y.wrapping_add(sector.quads_y));
    buffer.sphere_center=std::array::from_fn(|k|(p[k]+(y[k]-p[k])*0.5)+(x[k]-p[k])*0.5);
    let d:[f32;3]=std::array::from_fn(|k|p[k]-buffer.sphere_center[k]);
    buffer.sphere_radius=((d[2]*d[2]+d[1]*d[1])+d[0]*d[0]).sqrt();
}
