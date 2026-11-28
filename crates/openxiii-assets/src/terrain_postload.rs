//! ATerrainInfo::PostLoad: one owned state passed to rendering/collision.
//! The sector metadata effects are handled separately.
#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::terrain::{self, AlphaMap, Layer, Sector, TerrainData};
use crate::terrain_sector_buffer::{self as sector_buffer, SectorBuffer};
use crate::terrain_update::{self as geometry, SectorBufferBackend, TerrainUpdateBackend};
use openxiii_package::{library::Library, ObjectRef, Package, Reader, Value};

#[derive(Debug, Clone)]
pub struct TerrainMap {
    pub width: i32,
    pub height: i32,
    pub format: u8,
    pub pixels: Vec<u8>,
}

/// Slots keep NULL and duplicate references. Sector objects are stored once,
/// and object-indexed buffers are shared by all slots referring to that object.
pub struct PostLoadedTerrain {
    pub data: TerrainData,
    pub sector_slots: Vec<Option<usize>>,
    pub owner_reference: i32,
    pub location: [f32; 3],
    pub scale: [f32; 3],
    pub old_heightmap: Vec<u8>,
    pub terrain_map: Option<TerrainMap>,
    pub layers: Vec<Layer>,
    pub maps: Vec<Option<AlphaMap>>,
    /// Existing geometry API name; values are the virtual UMaterial::RequiresSorting,
    /// which equals bAlphaTexture only when the material is a UTexture.
    pub alpha_texture: Vec<bool>,
    pub visible: Vec<u32>,
    pub turned: Vec<u32>,
    /// Native packed nibbles, preserved until a real ComputeQuadMaterials call.
    pub material_table: Vec<u8>,
    pub buffers: Vec<Option<SectorBuffer>>,
}

fn array(o: &openxiii_package::ObjectData, p: &Package, name: &str, stride: usize) -> Vec<u8> {
    let Some(Value::Raw(raw)) = o.get(p,name) else { return Vec::new() };
    let mut r=Reader::new(raw,0);
    let Some(n)=r.compact().ok().filter(|n|*n>=0) else { return Vec::new() };
    r.take(n as usize*stride).unwrap_or_default().to_vec()
}
fn words(raw: Vec<u8>) -> Vec<u32> { raw.chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap())).collect() }

/// Package inputs as read, including references before NULL filtering and the legacy
/// height buffer.
pub fn read(library: &mut Library, map_name: &str, map: &Package, bytes: &[u8], index: usize) -> Option<PostLoadedTerrain> {
    let object=map.object_data(bytes,index).ok()?;
    let export=&map.exports[index];let end=(export.serial_offset+export.serial_size) as usize;
    let (refs,mut data,_,native_visibility,old_heightmap)=terrain::read_terrain_native_for_postload(&bytes[object.end..end],map.header.version,map.header.licensee).ok()?;
    let mut identities=std::collections::HashMap::new();let mut sector_slots=Vec::with_capacity(refs.len());
    for reference in refs {
        if reference==0 {sector_slots.push(None);continue}
        let slot=if let Some(&slot)=identities.get(&reference) {slot} else {
            let sector=match ObjectRef::from(reference) {
                ObjectRef::Export(i)=>terrain::read_sector(map,bytes,i).ok()?,
                _=> {let path=crate::world::qualified(map_name,map,reference);let (loaded,i)=library.resolve(&path)?;terrain::read_sector(&loaded.package,&loaded.data,i).ok()?},
            };
            let slot=data.sectors.len();data.sectors.push(sector);identities.insert(reference,slot);slot
        };
        sector_slots.push(Some(slot));
    }
    let terrain_map=match object.get(map,"TerrainMap") {
        Some(Value::Object(r)) if *r!=0=> {
            let path=crate::world::qualified(map_name,map,*r);
            let (loaded,i)=library.resolve(&path)?;let props=loaded.package.object_data(&loaded.data,i).ok()?;
            let texture=crate::texture::Texture::read(&loaded.package,&loaded.data,i).ok()?;
            let mip=texture.mips.first()?;
            let int=|name,default|match props.get(&loaded.package,name){Some(Value::Int(v))=>*v,_=>default};
            let format=match props.get(&loaded.package,"Format"){Some(Value::Byte(v))=>*v,_=>0};
            Some(TerrainMap{width:int("USize",mip.width as i32),height:int("VSize",mip.height as i32),format,pixels:mip.data.clone()})
        },
        _=>None,
    };
    let vector=|name,default|match object.get(map,name){Some(Value::Vector(v))=>*v,_=>default};
    let layers=terrain::read_layers(map_name,map,&object);
    // Calls the material's virtual RequiresSorting before testing opacity.
    let mut materials=crate::material::Materials::new();
    let alpha_texture=layers.iter().map(|l|materials.requires_sorting(library,&l.texture)).collect();
    Some(PostLoadedTerrain{data,sector_slots,owner_reference:index as i32+1,location:vector("Location",[0.0;3]),scale:vector("TerrainScale",[64.0;3]),old_heightmap,terrain_map,layers,maps:Vec::new(),alpha_texture,
        visible:native_visibility.unwrap_or_else(||words(array(&object,map,"QuadVisibilityBitmap",4))),turned:words(array(&object,map,"EdgeTurnBitmap",4)),material_table:array(&object,map,"QuadMaterialTable",1),buffers:Vec::new()})
}

impl PostLoadedTerrain {
    /// World resolves through the same identity table used by the update backend.
    pub fn buffer_for_slot(&self, slot:usize)->Option<&SectorBuffer>{
        self.sector_slots.get(slot).copied().flatten().and_then(|object|self.buffers.get(object)).and_then(Option::as_ref)
    }
    pub fn quad_materials(&self) -> Vec<u8> {
        self.material_table.iter().flat_map(|b|[b>>4,b&15]).collect()
    }
    fn compute_quad_materials(&mut self) {
        let material=self.data.quad_materials(&self.maps);
        self.material_table=material.chunks_exact(2).map(|v|(v[0]<<4)|v[1]).collect();
    }
    /// Parent repair predicate and call order, with the two distinct Q calls.
    pub fn post_load(&mut self, library: &mut Library, log: impl FnMut(String)) -> Result<(), &'static str> {
        self.post_load_with(&mut Production{library,log})
    }

    /// ATerrainInfo::PostLoad after AActor::PostLoad (reimplemented with actor loading): own body here,
    /// callees through `calls`.
    pub fn post_load_with<C: PostLoadCalls>(&mut self, calls: &mut C) -> Result<(), &'static str> {
        let coords:[f32;12]=[self.data.to_world.origin,self.data.to_world.x,self.data.to_world.y,self.data.to_world.z].concat().try_into().unwrap();
        // Centre (HX/2, HY/2, 32767) through +0x7B0, squared distance > 5: two GLog Logf calls, Location reset.
        if let Some(c)=terrain::post_load_origin(self.data.heightmap_x,self.data.heightmap_y,&coords,self.location) {
            for line in terrain::post_load_origin_log(c) {calls.log(line);}
            self.location=c;
        }
        // Non-empty OldHeightmap: G16 texture (Format 10) "%sHeightMap", Init(HX, HY), copy of HX × HY × 2 bytes,
        // OldHeightmap.Empty.
        if !self.old_heightmap.is_empty() {
            let count=self.data.heightmap_x.checked_mul(self.data.heightmap_y).filter(|n|*n>=0).ok_or("invalid legacy terrain dimensions")? as usize*2;
            let pixels=self.old_heightmap.get(..count).ok_or("short legacy heightmap")?.to_vec();
            self.terrain_map=Some(TerrainMap{width:self.data.heightmap_x,height:self.data.heightmap_y,format:10,pixels});
            self.old_heightmap=Vec::new();
        }
        // Bitmaps and material table sized to the height map.
        (self.visible,self.turned)=terrain::post_load_bitmaps(std::mem::take(&mut self.visible),std::mem::take(&mut self.turned),self.data.heightmap_x,self.data.heightmap_y);
        let count=terrain::post_load_quad_material_len(self.data.heightmap_x,self.data.heightmap_y).max(0) as usize;
        if self.material_table.len()!=count {self.material_table=vec![0;count];}
        // HX ≠ 8 × SX or HY ≠ 8 × SY: SetupSectors, CalcCoords, load of mip 0, Update(0, 0, 0, 0, 0, 0).
        let repair=self.data.heightmap_x!=self.data.sectors_x.wrapping_mul(8)||self.data.heightmap_y!=self.data.sectors_y.wrapping_mul(8);
        if repair {
            calls.setup_sectors(self);
            calls.calc_coords(self);
            calls.load_terrain_map(self)?;
            calls.update(self);
        }
        // ComputeQuadMaterials, always.
        calls.compute_quad_materials(self,repair);
        Ok(())
    }
}

/// Name of the G16 texture made from OldHeightmap (FString::Printf("%sHeightMap", GetName()), FName
/// with FNAME_Add), its Outer being Level->GetOuter().
pub fn heightmap_texture_name(actor: &str) -> String {
    format!("{actor}HeightMap")
}

/// Callees of ATerrainInfo::PostLoad outside its own body.
pub trait PostLoadCalls {
    /// GLog->Logf (import), line with its "Log: " tag.
    fn log(&mut self, line: String);
    /// ATerrainInfo::SetupSectors.
    fn setup_sectors(&mut self, t: &mut PostLoadedTerrain);
    /// ATerrainInfo::CalcCoords.
    fn calc_coords(&mut self, t: &mut PostLoadedTerrain);
    /// TerrainMap->Mips(0).DataArray.Load() (dereferences TerrainMap).
    fn load_terrain_map(&mut self, t: &mut PostLoadedTerrain) -> Result<(), &'static str>;
    /// ATerrainInfo::Update(0, 0, 0, 0, 0, 0).
    fn update(&mut self, t: &mut PostLoadedTerrain);
    /// ATerrainInfo::ComputeQuadMaterials.
    fn compute_quad_materials(&mut self, t: &mut PostLoadedTerrain, repaired: bool);
}

struct Production<'a,F>{library:&'a mut Library,log:F}
impl<F:FnMut(String)> PostLoadCalls for Production<'_,F>{
    fn log(&mut self,line:String){(self.log)(line)}
    fn setup_sectors(&mut self,t:&mut PostLoadedTerrain){
        let mut setup=crate::terrain_sectors::SetupSectorsState{heightmap:[t.data.heightmap_x,t.data.heightmap_y],sector_counts:[t.data.sectors_x,t.data.sectors_y],sectors:std::mem::take(&mut t.sector_slots),visible:std::mem::take(&mut t.visible),turned:std::mem::take(&mut t.turned),quad_materials:std::mem::take(&mut t.material_table)};
        let objects=&mut t.data.sectors;let owner=t.owner_reference;let log=&mut self.log;
        crate::terrain_sectors::setup_sectors(&mut setup,t.terrain_map.as_ref().map(|m|[m.width,m.height]),|init| {
            let mut sector=Sector{terrain_reference:0,quads_x:0,quads_y:0,offset_x:0,offset_y:0,bounds:[[0.0;3];8],colors:Vec::new(),lights:Vec::new(),color_revision:0};
            crate::terrain_sector_init::initialize_sector(&mut sector,owner,init.offset_x,init.offset_y,init.quads_x,init.quads_y);
            let index=objects.len();objects.push(sector);Some(index)
        },||log("expanding edgeturn bitmap".into()));
        [t.data.heightmap_x,t.data.heightmap_y]=setup.heightmap;[t.data.sectors_x,t.data.sectors_y]=setup.sector_counts;
        t.sector_slots=setup.sectors;t.visible=setup.visible;t.turned=setup.turned;t.material_table=setup.quad_materials;
    }
    fn calc_coords(&mut self,t:&mut PostLoadedTerrain){
        crate::terrain_coords::calc_coords(&mut t.data,t.location,t.scale,t.terrain_map.is_some());
    }
    fn load_terrain_map(&mut self,t:&mut PostLoadedTerrain)->Result<(),&'static str>{
        // Original dereferences TerrainMap/mip and every slot here. Reject malformed
        // input rather than treating a NULL slot as a fabricated geometric sector.
        if t.terrain_map.is_none()||t.sector_slots.iter().any(Option::is_none){return Err("NULL in terrain repair path")}
        Ok(())
    }
    fn update(&mut self,t:&mut PostLoadedTerrain){
        geometry::update(&mut ParentBackend{state:t,library:self.library},[0;6]);
    }
    fn compute_quad_materials(&mut self,t:&mut PostLoadedTerrain,repaired:bool){
        // Resident alpha data are also required by the final Q on non-repair paths.
        if !repaired {
            let library=&mut *self.library;
            t.maps=t.layers.iter().map(|l|match (&l.texture,&l.alpha_map){(Some(_),Some(path))=>terrain::load_alpha_map(library,path),_=>None}).collect();
        }
        t.compute_quad_materials();
    }
}

struct ParentBackend<'a>{state:&'a mut PostLoadedTerrain,library:&'a mut Library}
impl TerrainUpdateBackend for ParentBackend<'_>{
    fn heightmap_x(&self)->i32{self.state.data.heightmap_x}
    fn heightmap_y(&self)->i32{self.state.data.heightmap_y}
    fn calc_vertices(&mut self,a:[u32;5]){
        let s=&mut self.state;let layers=&s.layers;let maps=&mut s.maps;let library=&mut self.library;
        let heightmap=s.terrain_map.as_ref().map(|m|geometry::Heightmap{format:m.format,width:m.width,pixels:&m.pixels});
        geometry::calc_vertices(&mut s.data,heightmap,a[0],geometry::Rectangle{min_x:a[1]as i32,min_y:a[2]as i32,max_x:a[3]as i32,max_y:a[4]as i32},layers,|i|{
            if maps.len()<=i{maps.resize_with(i+1,||None)}
            maps[i]=layers[i].alpha_map.as_deref().and_then(|p|terrain::load_alpha_map(library,p));
        });
    }
    fn update_vertex_buffers(&mut self,a:[u32;5]){geometry::update_vertex_buffers(self,geometry::Rectangle{min_x:a[0]as i32,min_y:a[1]as i32,max_x:a[2]as i32,max_y:a[3]as i32},a[4]);}
    fn compute_quad_materials(&mut self){self.state.compute_quad_materials();}
}
impl SectorBufferBackend for ParentBackend<'_>{
    fn sector_count(&self)->usize{self.state.sector_slots.len()}
    fn sector_rectangle(&self,i:usize)->geometry::SectorRectangle{
        let s=&self.state.data.sectors[self.state.sector_slots[i].expect("validated slot")];
        geometry::SectorRectangle{quads_x:s.quads_x,quads_y:s.quads_y,offset_x:s.offset_x,offset_y:s.offset_y}
    }
    fn update_sector_buffer(&mut self,i:usize){
        let s=&mut self.state;let index=s.sector_slots[i].expect("validated slot");
        if s.buffers.len()<=index{s.buffers.resize_with(index+1,||None)}
        let mut sector=s.data.sectors[index].clone();
        let buffer=s.buffers[index].get_or_insert_with(SectorBuffer::default);
        sector_buffer::update_sector_buffer(&s.data,&mut sector,buffer,sector_buffer::Inputs{
            heightmap:s.terrain_map.as_ref().map(|m|geometry::Heightmap{format:m.format,width:m.width,pixels:&m.pixels}),
            layers:&s.layers,maps:&s.maps,alpha_texture:&s.alpha_texture,visible:&s.visible,turned:&s.turned});
        s.data.sectors[index]=sector;
    }
    fn static_light(&mut self,_:usize,_:u32){unreachable!("PostLoad always supplies lighting flag zero")}
}
