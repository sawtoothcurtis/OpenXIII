//! Extraction of the existing CPU consumer, without GPU or policy change.
use super::{DrawBits,PropLight};

pub(super) fn prop_ambient(
    p:&PropLight,
    model:Option<&openxiii_assets::model::Model>,
    pass:Option<&crate::lights_scene::Pass>,
    draw:Option<&crate::lights_scene::Draw>,
    region_nodes:&[openxiii_assets::world::RegionNode],
    root_outside:bool,
    hsv:&dyn Fn(usize)->[f32;3],
)->([u8;3],Vec<i32>,Vec<Vec<usize>>,i32) {
    let mut zones:Vec<i32>=Vec::new();
    let mut leaf_lights:Vec<Vec<usize>>=Vec::new();
    if let Some(model)=model {
        for &l in &p.leaves {
            if let Some(&z)=model.leaves.get(l as usize) {
                zones.push(z);
                leaf_lights.push(match pass {
                    Some(pass)=>pass.leaf_lights(model,l as usize),
                    None=>model.leaf_lights(l as usize),
                });
            }
        }
    }
    let pivot=openxiii_assets::world::point_zone(region_nodes,root_outside,p.location)as i32;
    let list:Vec<usize>=if p.pivot_ambient||zones.len()<=1 {vec![pivot.max(0)as usize]}else{zones.iter().map(|&z|z.max(0)as usize).collect()};
    let mut seen:Vec<usize>=Vec::new();
    for z in list {
        if !seen.contains(&z)&&seen.len()<4 {seen.push(z);}
    }
    let mut m=[0.0f32;3];
    for z in seen {
        let c=hsv(z);
        for k in 0..3 {
            if m[k]<c[k] {m[k]=c[k];}
        }
    }
    (draw.and_then(|d|d.refresh.as_ref()).map_or_else(||openxiii_assets::lighting::ambient_color(m,p.glow),|r|r.ambient),zones,leaf_lights,pivot)
}

pub(super) fn prop_light_guard(p:&PropLight)->Option<(DrawBits,Vec<[[f32;4];4]>)> {
    if p.unlit {return Some((DrawBits::default(),Vec::new()));}
    let has_color=p.instance.is_some();
    if p.ignore_dyn {return Some((DrawBits{color_vertex:true,has_color,..Default::default()},Vec::new()));}
    None
}

