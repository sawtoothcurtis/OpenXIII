fn set_character_screen_url(vm: &mut Vm, url: &openxiii_vm::travel::Url) {
    vm.character_screen_from_url(url);
}

fn character_camera(vm: &Vm, actor: Option<ObjId>, location: &mut [f32; 3], rotation: &mut [i32; 3]) -> Option<ObjId> {
    match vm.character_screen_view(0) {
        Some((l, r)) => {
            (*location, *rotation) = (l, r);
            None
        }
        None => actor,
    }
}

fn player_arms_mesh(vm: &Vm, actor: ObjId) -> Option<&str> {
    vm.arms_mesh_override(actor)
}

fn enhanced_shadow_settings(vm: &Vm) -> bool {
    vm.engine.client.session_graphics.enhanced()
}
