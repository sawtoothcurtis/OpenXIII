//! CPU transformation actually called by Renderer::set_dyn_lights.
use openxiii_assets::{lighting::Light,model::LightRecord};
pub(super) fn refresh_value(lights:&mut Vec<Light>,model_lights:usize,records:&[LightRecord],brightness:f32,time:f64,count:u32) {
    lights.truncate(model_lights);
    let mut seed=count;
    let mut frand=|| {
        seed=seed.wrapping_mul(214_013).wrapping_add(2_531_011);
        ((seed>>16)&0x7FFF) as f32/32767.0
    };
    for r in records { lights.push(openxiii_assets::lighting::init_light(r,brightness,time,&mut frand)); }
}
