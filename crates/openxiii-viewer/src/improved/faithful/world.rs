fn set_character_screen_url(_vm: &mut Vm, _url: &openxiii_vm::travel::Url) {}

fn character_camera(_vm: &Vm, actor: Option<ObjId>, _location: &mut [f32; 3], _rotation: &mut [i32; 3]) -> Option<ObjId> {
    actor
}

fn player_arms_mesh(_vm: &Vm, _actor: ObjId) -> Option<&str> {
    None
}

fn enhanced_shadow_settings(_: &Vm) -> bool {
    false
}
