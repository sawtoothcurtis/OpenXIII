//! Presentation defaults for the original animation, camera and audio paths.
use crate::anim::MeshInfo;
use crate::skel::PoseState;
use crate::value::ObjId;
use crate::Vm;
use openxiii_assets::coords::Coords;
use openxiii_hx::subsystem::SoundRef;

macro_rules! pose_state {
    ($item:item) => { $item };
}
pub(crate) use pose_state;

pub(crate) fn finish_pawn_gesture(_vm: &mut Vm, _actor: ObjId, _channel: i32) {}

pub(crate) fn bone_rotation(_vm: &mut Vm, _pawn: ObjId, _bone: &str, _head: bool, _dt: f32, rotation: [i32; 3]) -> [i32; 3] {
    rotation
}

pub(crate) fn finish_pose(_pose: &PoseState, _info: &MeshInfo, _bases: &mut [Coords]) {}

pub(crate) fn menu_sound(_vm: &mut Vm, _path: &str) -> Option<Option<SoundRef>> {
    None
}

pub(crate) fn character_camera(_vm: &Vm, _slot: u8) -> Option<([f32; 3], [i32; 3])> {
    None
}

pub(crate) fn prepare_character_view(_vm: &mut Vm, _slot: u8) {}

pub(crate) fn scene_framing(_vm: &Vm, _slot: u8, _rect: [i32; 4], fov: f32) -> (f32, Option<[i32; 4]>, [f32; 2], f32) {
    (fov, None, [0.0; 2], fov)
}

pub(crate) fn hidden_projectors(_vm: &Vm, _slot: u8) -> Vec<ObjId> {
    Vec::new()
}
