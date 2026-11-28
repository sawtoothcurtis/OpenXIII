//! Presentation adjustments for character selection, co-op poses and menu sound.
use crate::anim::MeshInfo;
use crate::skel::PoseState;
use crate::value::ObjId;
use crate::Vm;
use openxiii_assets::coords::{self, Coords, UNIT};
use openxiii_hx::subsystem::SoundRef;
#[allow(unused_imports)]
use openxiii_math::SimMath as _;

// Preserve the public body_turn field only in the edition that implements it.
macro_rules! pose_state {
    ($(#[$attr:meta])* $vis:vis struct $name:ident { $($fields:tt)* }) => {
        $(#[$attr])*
        $vis struct $name {
            $($fields)*
            /// World yaw in Unreal units, applied around the pelvis after posing.
            pub body_turn: i32,
        }
    };
}
pub(crate) use pose_state;

pub(crate) fn finish_pawn_gesture(vm: &mut Vm, actor: ObjId, channel: i32) {
    vm.coop_item_gesture_end(actor, channel);
}

pub(crate) fn bone_rotation(vm: &mut Vm, pawn: ObjId, bone: &str, head: bool, dt: f32, rotation: [i32; 3]) -> [i32; 3] {
    let rotation = vm.coop_look_bone(pawn, head, dt, rotation);
    vm.coop_lean_limited(pawn, bone, rotation)
}

pub(crate) fn finish_pose(pose: &PoseState, info: &MeshInfo, bases: &mut [Coords]) {
    if pose.body_turn != 0 {
        turn_skeleton(bases, info, &pose.scale_coords, pose.body_turn);
    }
}

pub(crate) fn menu_sound(vm: &mut Vm, path: &str) -> Option<Option<SoundRef>> {
    vm.menu_wave_ref(path)
}

pub(crate) fn character_camera(vm: &Vm, slot: u8) -> Option<([f32; 3], [i32; 3])> {
    vm.character_screen_view(slot)
}

pub(crate) fn prepare_character_view(vm: &mut Vm, slot: u8) {
    vm.character_screen_for_view(slot);
}

/// Draw the scene within the player's live-view panel using its recorded field of view.
pub(crate) fn scene_framing(vm: &Vm, slot: u8, rect: [i32; 4], fov: f32) -> (f32, Option<[i32; 4]>, [f32; 2], f32) {
    match vm.character_screen_scene(slot) {
        Some(v) => (v.fov, Some([rect[0] + v.panel[0], rect[1] + v.panel[1], v.panel[2], v.panel[3]]), v.lens, v.cull_fov),
        None => (fov, None, [0.0; 2], fov),
    }
}

pub(crate) fn hidden_projectors(vm: &Vm, slot: u8) -> Vec<ObjId> {
    vm.character_screen_hidden_shadows(slot)
}

/// Improved build only: every bone of `bases` (frames in the mesh's frame `scale`) turned by
/// `yaw` around the world's vertical through the pelvis. The turn in the mesh's frame is the
/// one SetBoneDirection's world directions take (as in the composition above): a direction
/// given there with its yaw less `yaw` ends up exactly where it was asked.
pub(crate) fn turn_skeleton(bases: &mut [Coords], info: &MeshInfo, scale: &Coords, yaw: i32) {
    let frame = |r: [i32; 3]| -> [[f32; 3]; 3] {
        let m = scale.transpose().mul(&coords::div_rotator(&UNIT, r));
        let n = |a: [f32; 3]| {
            let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
            if l > 1e-6 { a.map(|c| c / l) } else { a }
        };
        [n(m.x), n(m.y), n(m.z)]
    };
    // A = F(yaw) · F(0)⁻¹, F a direction's frame (rows): the same for every direction.
    let (f0, f1) = (frame([0; 3]), frame([0, yaw, 0]));
    let Some(inv) = invert3(f0) else { return };
    let a: [[f32; 3]; 3] = [0, 1, 2].map(|i| [0, 1, 2].map(|j| (0..3).map(|k| f1[i][k] * inv[k][j]).sum()));
    let pelvis = crate::skel::match_ref_bone(info, "X Pelvis");
    let pivot = bases.get(if pelvis >= 0 { pelvis as usize } else { 0 }).map(|b| b.origin).unwrap_or([0.0; 3]);
    let apply = |v: [f32; 3]| [0, 1, 2].map(|i| a[i][0] * v[0] + a[i][1] * v[1] + a[i][2] * v[2]);
    for b in bases.iter_mut() {
        // Rows of A · B (a bone's frame maps its points through its rows: skinning).
        let (x, y, z) = (b.x, b.y, b.z);
        let row = |r: [f32; 3]| [0, 1, 2].map(|j| r[0] * x[j] + r[1] * y[j] + r[2] * z[j]);
        (b.x, b.y, b.z) = (row(a[0]), row(a[1]), row(a[2]));
        let d = apply([b.origin[0] - pivot[0], b.origin[1] - pivot[1], b.origin[2] - pivot[2]]);
        b.origin = [pivot[0] + d[0], pivot[1] + d[1], pivot[2] + d[2]];
    }
}

fn invert3(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let c = |i: usize, j: usize| {
        let (a, b) = ((i + 1) % 3, (i + 2) % 3);
        let (p, q) = ((j + 1) % 3, (j + 2) % 3);
        m[a][p] * m[b][q] - m[a][q] * m[b][p]
    };
    let det = m[0][0] * c(0, 0) + m[0][1] * c(0, 1) + m[0][2] * c(0, 2);
    if det.abs() < 1e-12 {
        return None;
    }
    Some([0, 1, 2].map(|i| [0, 1, 2].map(|j| c(j, i) / det)))
}
