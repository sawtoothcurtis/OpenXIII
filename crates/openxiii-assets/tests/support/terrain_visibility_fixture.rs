//! Embeds the Serialize visibility case in a valid non-repair PostLoad context.
//! The original input data and expected collision bytes are never modified.
use openxiii_package::{Export,Header,Import,Name,Package,Reader};

pub fn package(version:u16,licensee:u16,native:&[u8],visible:&[u8])->(Package,Vec<u8>){
    let (_,before,end,before_visibility,_)=openxiii_assets::terrain::read_terrain_native_for_postload(native,version,licensee).unwrap();
    assert_eq!(end,native.len());assert_eq!((before.heightmap_x,before.heightmap_y),(8,8));
    // These input fields are not checked by this visibility test's Serialize comparison:
    // sector counts were randomized and old height arrays intentionally short.
    // Keep exact vertices, matrices, native visibility and every other serialized byte.
    let mut r=Reader::new(native,0);assert_eq!(r.compact().unwrap(),0);
    let n=r.compact().unwrap() as usize;r.take(n*12).unwrap();let counts=r.pos;r.take(8).unwrap();
    if licensee<20{let n=r.compact().unwrap()as usize;r.take(n*24).unwrap();}
    if version<83{let n=r.compact().unwrap()as usize;r.take(n*16).unwrap();}
    r.take(96).unwrap();let mut focused=native.to_vec();
    if version<76{let start=r.pos;let n=r.compact().unwrap()as usize;r.take(n*2).unwrap();focused.splice(start..r.pos,[0]);}
    focused[counts..counts+4].copy_from_slice(&1i32.to_le_bytes());focused[counts+4..counts+8].copy_from_slice(&1i32.to_le_bytes());
    let (_,after,end,after_visibility,old)=openxiii_assets::terrain::read_terrain_native_for_postload(&focused,version,licensee).unwrap();
    assert_eq!(end,focused.len());assert!(old.is_empty());assert_eq!(before.vertices,after.vertices);assert_eq!(before_visibility,after_visibility);
    assert_eq!(before.to_world,after.to_world);assert_eq!(before.to_heightmap,after.to_heightmap);assert_eq!(before.sounds,after.sounds);assert_eq!(before.noise_loudness.to_bits(),after.noise_loudness.to_bits());
    // TerrainMap now references export2, a real 8x8 G16 Texture with one mip.
    let mut data=vec![5,0x05,2,6,0x59,(visible.len()+1)as u8,(visible.len()/4)as u8];
    data.extend(visible);data.push(0);data.extend(focused);let terrain_size=data.len();
    let mut texture=vec![9,0x01,10,10,0x22];texture.extend(8i32.to_le_bytes());texture.extend([11,0x22]);texture.extend(8i32.to_le_bytes());texture.push(0);
    texture.push(1);texture.extend(0u32.to_le_bytes());texture.extend([0x40,2]);texture.extend([0u8;128]);texture.extend(8i32.to_le_bytes());texture.extend(8i32.to_le_bytes());texture.extend([3,3]);
    let texture_size=texture.len();data.extend(texture);
    let names=["None","TerrainInfo","Class","Engine","Core","TerrainMap","QuadVisibilityBitmap","Texture","Height","Format","USize","VSize"].map(|n|Name{name:n.into(),flags:0x30000}).to_vec();
    let package=Package{header:Header{version,licensee,flags:0,name_count:names.len()as u32,name_offset:0,export_count:2,export_offset:0,import_count:2,import_offset:0,guid:[0;16],generations:Vec::new()},names,
        imports:vec![Import{class_package:3,class_name:2,package:0,object_name:1},Import{class_package:3,class_name:2,package:0,object_name:7}],
        exports:vec![Export{class:-1,super_:0,package:0,object_name:1,flags:0,serial_offset:0,serial_size:terrain_size as i32},Export{class:-2,super_:0,package:0,object_name:8,flags:0,serial_offset:terrain_size as i32,serial_size:texture_size as i32}]};
    (package,data)
}
