//! Lockstep co-op, drawing only: the HUD's icons over a world point. A step makes this
//! machine's view (its HUD included) with the step's camera, and the frame draws the scene with
//! the camera of its own moment (the instant look's turn, the place between the last two steps,
//! lockstep_play.rs): an icon put by the HUD script where Console.WorldToScreen projected a world
//! point (a grapple's hook point, a hint frame, a speaker's arrow) would stay where the step's
//! camera saw that point and float over the scene. Each such projection is kept with the
//! primitives the script draws after it (`HudAnchor`), and the frame moves them by what that
//! point moves on the screen from the step's camera to the drawn one (`shift_hud`). The game
//! never sees any of it: WorldToScreen returns what it always returned.

use crate::canvas::HudDraw;
use crate::Vm;
use openxiii_assets::coords::Matrix;

/// The script functions whose WorldToScreen marks a world point the primitives drawn after it
/// stand on: Hook.OutlinePotentialTarget (the hook points' icons), HudCartoonFocus.DrawWnd (the
/// hint frame), HudDialog.DrawDlg (the speaker's arrow), XIIIPlayerInteraction.DrawInteractions
/// (the focus outline of an object to use). Left out: XIIIPlayerInteraction.DrawWeaponCrosshair
/// (the floating crosshair marks the aim, which turns with the view) and SixSenseSkill.Heard
/// (its point is kept and drawn later, where the noise was heard).
pub(crate) const ANCHORED: [(&str, &str); 4] = [("Hook", "OutlinePotentialTarget"), ("HudCartoonFocus", "DrawWnd"), ("HudDialog", "DrawDlg"), ("XIIIPlayerInteraction", "DrawInteractions")];

/// A world point a HUD script projected, and the HUD primitives drawn on it.
#[derive(Debug, Clone)]
pub struct HudAnchor {
    /// The primitives `start..end` of the view's HUD.
    pub start: usize,
    pub end: usize,
    world: [f32; 3],
    /// The camera WorldToScreen projected with (location, rotation).
    camera: ([f32; 3], [i32; 3]),
    world_to_screen: Matrix,
    screen_to_canvas: Matrix,
    /// What WorldToScreen returned (canvas units).
    screen: [f32; 3],
    /// Canvas units to viewport pixels (the improved build's HUD scale), else 1.
    pub(crate) scale: [f32; 2],
}

/// A rotation's forward, right and up axes (Unreal's FRotationMatrix, roll included).
pub fn rotation_axes(rot: [i32; 3]) -> [[f32; 3]; 3] {
    let rad = |u: i32| (u as f32) * std::f32::consts::PI / 32768.0;
    let (sp, cp) = rad(rot[0]).sin_cos();
    let (sy, cy) = rad(rot[1]).sin_cos();
    let (sr, cr) = rad(rot[2]).sin_cos();
    [[cp * cy, cp * sy, sp], [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp], [-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl HudAnchor {
    /// How far (viewport pixels) the point moves on the screen when the frame's camera is
    /// `drawn` instead of the step's `step` (location, rotation); None when the drawn camera no
    /// longer has it in front.
    pub fn moved(&self, step: ([f32; 3], [i32; 3]), drawn: ([f32; 3], [i32; 3])) -> Option<[f32; 2]> {
        // The anchor's own camera moved as the frame's camera moved from the step's.
        let at = [0, 1, 2].map(|i| self.camera.0[i] + (drawn.0[i] - step.0[i]));
        let rot = [0, 1, 2].map(|i| self.camera.1[i].wrapping_add(drawn.1[i].wrapping_sub(step.1[i])));
        // The point as the moved camera sees it, put where the anchor's camera would see it so,
        // projected with the very matrix WorldToScreen used.
        let d = [0, 1, 2].map(|i| self.world[i] - at[i]);
        let local = rotation_axes(rot).map(|a| dot(a, d));
        if local[0] <= 0.0 {
            return None;
        }
        let axes = rotation_axes(self.camera.1);
        let seen = [0, 1, 2].map(|k| self.camera.0[k] + axes[0][k] * local[0] + axes[1][k] * local[1] + axes[2][k] * local[2]);
        let p = crate::visibility::project_with(&self.world_to_screen, seen);
        if p[3] <= 0.0 {
            return None;
        }
        let v = openxiii_assets::coords::transform_fvector(&self.screen_to_canvas, [p[0], p[1], p[2]]);
        Some([(v[0] - self.screen[0]) * self.scale[0], (v[1] - self.screen[1]) * self.scale[1]])
    }
}

/// A HUD primitive moved by `d` pixels.
fn shift(d: &mut HudDraw, by: [f32; 2]) {
    match d {
        HudDraw::Tile { x1, y1, x2, y2, .. } => {
            *x1 += by[0];
            *x2 += by[0];
            *y1 += by[1];
            *y2 += by[1];
        }
        HudDraw::Triangle { p, .. } => {
            for q in p.iter_mut() {
                q[0] += by[0];
                q[1] += by[1];
            }
        }
        HudDraw::Line { from, to, .. } => {
            from[0] += by[0];
            from[1] += by[1];
            to[0] += by[0];
            to[1] += by[1];
        }
        HudDraw::Portal { rect, .. } => {
            rect[0] += by[0];
            rect[1] += by[1];
        }
    }
}

/// The frame's HUD: each anchor's primitives moved onto the point as the drawn camera sees it
/// (`step`: the camera the step made the view with). Returns how many primitives moved.
pub fn shift_hud(hud: &mut [HudDraw], anchors: &[HudAnchor], step: ([f32; 3], [i32; 3]), drawn: ([f32; 3], [i32; 3])) -> usize {
    if step == drawn {
        return 0;
    }
    let mut n = 0;
    for a in anchors {
        let Some(by) = a.moved(step, drawn) else { continue };
        let end = a.end.min(hud.len());
        for d in hud.get_mut(a.start.min(end)..end).into_iter().flatten() {
            shift(d, by);
            n += 1;
        }
    }
    n
}

impl Vm {
    /// UInteraction::execWorldToScreen in a lockstep session: when one of the `ANCHORED`
    /// functions projects `world`, the primitives it draws next stand on that point.
    pub(crate) fn hud_anchor_open(&mut self, world: [f32; 3], camera: ([f32; 3], [i32; 3]), node: &crate::visibility::SceneNode, screen_to_canvas: Matrix, screen: [f32; 3]) {
        if self.lockstep.is_none() {
            return;
        }
        // The script function that called the native (natives may have a frame of their own).
        let native = |f: usize| self.functions[f].native != 0 || self.functions[f].flags & openxiii_script::objects::func::NATIVE != 0;
        let Some((depth, &f)) = self.stack.iter().enumerate().rev().find(|(_, &f)| !native(f)) else { return };
        let func = &self.functions[f];
        let class = &self.classes[func.owner].name;
        if !ANCHORED.iter().any(|(c, n)| class.eq_ignore_ascii_case(c) && func.name.eq_ignore_ascii_case(n)) {
            return;
        }
        self.hud_anchors.push(HudAnchor { start: self.hud.len(), end: usize::MAX, world, camera, world_to_screen: node.world_to_screen_matrix(), screen_to_canvas, screen, scale: [1.0, 1.0] });
        self.hud_anchor_depth = Some(depth + 1);
    }

    /// The open anchor (if any) ends with the primitives drawn so far.
    pub(crate) fn hud_anchor_close(&mut self) {
        self.hud_anchor_depth = None;
        let len = self.hud.len();
        if let Some(a) = self.hud_anchors.last_mut().filter(|a| a.end == usize::MAX) {
            a.end = len;
        }
    }

    /// The view's anchors, with its HUD (ViewportFrame::anchors).
    pub(crate) fn hud_anchors_take(&mut self) -> Vec<HudAnchor> {
        self.hud_anchor_close();
        std::mem::take(&mut self.hud_anchors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unreal's axes: yaw 16384 looks along +Y, pitch 16384 up; roll turns the right axis down.
    #[test]
    fn axes_of_rotations() {
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4);
        assert!(close(rotation_axes([0, 16384, 0])[0], [0.0, 1.0, 0.0]));
        assert!(close(rotation_axes([16384, 0, 0])[0], [0.0, 0.0, 1.0]));
        let r = rotation_axes([0, 0, 16384]);
        assert!(close(r[1], [0.0, 0.0, -1.0]) && close(r[2], [0.0, 1.0, 0.0]), "{r:?}");
        // Without roll, the axes of lockstep_interp's camera space.
        let a = rotation_axes([3000, 12000, 0]);
        let b = crate::lockstep_interp::camera_axes([3000, 12000, 0]);
        for k in 0..3 {
            assert!(close(a[k], b[k]));
        }
    }

    /// A simple pinhole: x right, y down from the centre of a 1000 × 1000 canvas, focal 500.
    fn pinhole(camera: ([f32; 3], [i32; 3]), world: [f32; 3]) -> HudAnchor {
        // World to camera axes (forward, right, up), then a projection that puts (right/forward,
        // up/forward) on the screen: X = 500 + 500 r / f, Y = 500 − 500 u / f (W = f).
        let a = rotation_axes(camera.1);
        let t = [0, 1, 2].map(|k| -dot(a[k], camera.0));
        // Row-vector matrix (p × M), as transform_fplane reads it: column j = output j.
        let col = |j: usize, k: usize| match j {
            0 => 500.0 * a[1][k] + 500.0 * a[0][k],
            1 => -500.0 * a[2][k] + 500.0 * a[0][k],
            2 => 0.0,
            _ => a[0][k],
        };
        let tr = |j: usize| match j {
            0 => 500.0 * t[1] + 500.0 * t[0],
            1 => -500.0 * t[2] + 500.0 * t[0],
            2 => 1.0,
            _ => t[0],
        };
        let mut m = [0.0f32; 16];
        for j in 0..4 {
            for k in 0..3 {
                m[k * 4 + j] = col(j, k);
            }
            m[12 + j] = tr(j);
        }
        let mut id = [0.0f32; 16];
        for i in 0..4 {
            id[i * 5] = 1.0;
        }
        let p = crate::visibility::project_with(&m, world);
        HudAnchor { start: 0, end: 1, world, camera, world_to_screen: m, screen_to_canvas: id, screen: [p[0], p[1], p[2]], scale: [1.0, 1.0] }
    }

    /// The icon moves with the point as the drawn camera sees it: a point straight ahead stays
    /// in the middle without a turn; a turn to the left by the angle of a point 100 to the left
    /// at 500 ahead brings that point to the middle; a step forward towards a point off the axis
    /// moves it outwards; the step's camera itself moves nothing.
    #[test]
    fn anchors_follow_the_drawn_camera() {
        let cam = ([0.0, 0.0, 0.0], [0, 0, 0]);
        let ahead = pinhole(cam, [500.0, 0.0, 0.0]);
        assert!((ahead.screen[0] - 500.0).abs() < 1e-3 && (ahead.screen[1] - 500.0).abs() < 1e-3, "{:?}", ahead.screen);
        assert_eq!(ahead.moved(cam, cam).map(|d| d.map(|x| x.abs() < 1e-3)), Some([true, true]));
        let left = pinhole(cam, [500.0, -100.0, 0.0]);
        assert!((left.screen[0] - 400.0).abs() < 1e-2, "{:?}", left.screen);
        let yaw = -((100.0f32 / 500.0).atan() * 32768.0 / std::f32::consts::PI).round() as i32;
        let d = left.moved(cam, ([0.0; 3], [0, yaw, 0])).expect("in front");
        assert!((d[0] - 100.0).abs() < 0.5 && d[1].abs() < 0.5, "{d:?}");
        let off = pinhole(cam, [500.0, 100.0, 50.0]);
        let d = off.moved(cam, ([100.0, 0.0, 0.0], [0, 0, 0])).expect("in front");
        assert!(d[0] > 20.0 && d[1] < -10.0, "{d:?}");
        assert!(off.moved(cam, ([600.0, 0.0, 0.0], [0, 0, 0])).is_none(), "behind the drawn camera");
        // The frame's primitives on it, and one after it that is not.
        let tile = |x: f32| HudDraw::Tile { x1: x, y1: 10.0, x2: x + 8.0, y2: 18.0, uv: [0.0; 4], material: "m".into(), color: [255; 4], point: false, enlarged: false, glyph: false };
        let mut hud = vec![tile(395.0), tile(0.0)];
        let n = shift_hud(&mut hud, &[left.clone()], cam, ([0.0; 3], [0, yaw, 0]));
        let x = |d: &HudDraw| if let HudDraw::Tile { x1, .. } = d { *x1 } else { f32::NAN };
        assert_eq!(n, 1);
        assert!((x(&hud[0]) - 495.0).abs() < 0.5, "{}", x(&hud[0]));
        assert_eq!(x(&hud[1]), 0.0, "a primitive outside the anchor stays");
    }
}
