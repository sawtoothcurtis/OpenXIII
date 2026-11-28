//! Mods build only (feature `mods`): heat shimmer over an area of the world (`heat{}` of the
//! scripts). It goes through the game's own 2D effects path (the scene drawn into a texture,
//! then passes drawn back to the screen, screen_fx.rs): a grid laid over the area's place on the
//! screen whose inner points sample the scene a little to the side, waving with the level's time.
//! Its outer points sample the scene where they are, so the area has no visible edge. Nothing of
//! this exists without a script asking for it.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::screen_fx::{FxPass, FxVertex};
use crate::value::ObjId;
use crate::Vm;

/// Points of the grid along each side.
const GRID: u32 = 13;
/// Largest sideways shift of the shimmer, in parts of the screen, at strength 1.
const SHIFT: f32 = 0.004;
/// Areas drawn at most (the nearest to the view first).
const MAX_ZONES: usize = 12;

/// An area of shimmer: around a point or an actor, until a level time.
#[derive(Debug, Clone)]
pub struct HeatZone {
    pub name: String,
    pub at: [f32; 3],
    pub on: Option<ObjId>,
    pub radius: f32,
    pub strength: f32,
    pub until: f32,
}

impl Vm {
    /// `heat{}`: adds the area, or replaces the one of the same name.
    pub(crate) fn mod_heat_add(&mut self, zone: HeatZone) {
        self.mods.surfaces.set_heat_used(true);
        if !zone.name.is_empty() {
            if let Some(z) = self.mods.heat.iter_mut().find(|z| z.name == zone.name) {
                *z = zone;
                return;
            }
        }
        self.mods.heat.push(zone);
    }

    /// The passes of the shimmer for the view of `controller` from `origin`; `redirected`: the
    /// game's own effects already drew the scene back to the screen (otherwise a plain copy
    /// comes first). Empty without areas in sight.
    pub(crate) fn mod_heat_passes(&mut self, controller: Option<ObjId>, origin: Option<[f32; 3]>, redirected: bool) -> Vec<FxPass> {
        if self.mods.heat.is_empty() {
            return Vec::new();
        }
        let now = self.mod_level_time();
        let alive = |vm: &Vm, a: ObjId| vm.objects.get(a).is_some_and(|o| o.is_some());
        let objects_ok: Vec<bool> = self.mods.heat.iter().map(|z| z.on.is_none_or(|a| alive(self, a))).collect();
        let mut keep = objects_ok.into_iter();
        self.mods.heat.retain(|z| keep.next().unwrap_or(false) && z.until > now);
        let (Some(pc), Some(eye)) = (controller, origin) else { return Vec::new() };
        if self.objects.get(pc).is_none_or(|o| o.is_none()) {
            return Vec::new();
        }
        // The view: the controller's rotation and field of view (horizontal, degrees).
        let [pitch, yaw, _] = self.prop(pc, "Rotation").as_rotator();
        let fov = self.prop(pc, "FovAngle").as_float();
        let fov = if fov > 1.0 && fov < 179.0 { fov } else { 90.0 };
        let (w, h) = self.viewport_size();
        let aspect = if w > 0 && h > 0 { w as f32 / h as f32 } else { 4.0 / 3.0 };
        let unit = std::f32::consts::PI / 32768.0;
        let (p, y) = (pitch as f32 * unit, yaw as f32 * unit);
        let fwd = [p.dcos() * y.dcos(), p.dcos() * y.dsin(), p.dsin()];
        let right = [-y.dsin(), y.dcos(), 0.0];
        let up = [-p.dsin() * y.dcos(), -p.dsin() * y.dsin(), p.dcos()];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let tan = (fov.to_radians() * 0.5).dtan();
        // Each area in sight: its centre and radius on the screen ([-1, 1], y upwards).
        let mut shown: Vec<(f32, [f32; 2], [f32; 2], f32)> = Vec::new();
        for z in &self.mods.heat {
            let at = match z.on {
                Some(a) => {
                    let l = self.prop(a, "Location").as_vector();
                    [l[0] + z.at[0], l[1] + z.at[1], l[2] + z.at[2]]
                }
                None => z.at,
            };
            let d = [at[0] - eye[0], at[1] - eye[1], at[2] - eye[2]];
            let depth = dot(d, fwd);
            if depth < 10.0 {
                continue;
            }
            let sx = dot(d, right) / (depth * tan);
            let sy = dot(d, up) / (depth * tan) * aspect;
            let rx = z.radius / (depth * tan);
            let ry = rx * aspect;
            if sx + rx < -1.0 || sx - rx > 1.0 || sy + ry < -1.0 || sy - ry > 1.0 {
                continue;
            }
            shown.push((depth, [sx, sy], [rx.min(1.5), ry.min(1.5)], z.strength.clamp(0.0, 2.0)));
        }
        if shown.is_empty() {
            return Vec::new();
        }
        shown.sort_by(|a, b| a.0.total_cmp(&b.0));
        shown.truncate(MAX_ZONES);
        let mut out = Vec::new();
        if !redirected {
            // The scene back to the screen as it is (the game's copy, without blending).
            out.push(FxPass::Target(crate::screen_fx::FxTarget::Back));
            out.push(crate::screen_fx::copy_quad());
        }
        let uv = |x: f32, y: f32| [(x + 1.0) * 0.5, (1.0 - y) * 0.5];
        for (k, (_, c, r, strength)) in shown.into_iter().enumerate() {
            let mut verts = Vec::with_capacity((GRID * GRID) as usize);
            let last = (GRID - 1) as f32;
            for j in 0..GRID {
                for i in 0..GRID {
                    let (fx, fy) = (i as f32 / last * 2.0 - 1.0, j as f32 / last * 2.0 - 1.0);
                    let (x, y) = (c[0] + fx * r[0], c[1] + fy * r[1]);
                    // No shift on the edge, the most in the middle.
                    let edge = i == 0 || j == 0 || i == GRID - 1 || j == GRID - 1;
                    let fall = if edge { 0.0 } else { (1.0 - (fx * fx + fy * fy).sqrt()).max(0.0) };
                    let phase = now * 9.0 + k as f32 * 1.7;
                    let dx = (phase + fy * 6.0 + fx * 2.0).dsin() * SHIFT * strength * fall;
                    let dy = (phase * 1.3 + fx * 5.0).dcos() * SHIFT * 0.6 * strength * fall;
                    let t = uv(x, y);
                    let s = [t[0] + dx, t[1] + dy];
                    verts.push(FxVertex { pos: [x, y], uv0: s, uv1: s, color: 0xffff_ffff });
                }
            }
            let mut indices = Vec::with_capacity(((GRID - 1) * (GRID - 1) * 6) as usize);
            for j in 0..GRID - 1 {
                for i in 0..GRID - 1 {
                    let a = (j * GRID + i) as u16;
                    let b = a + 1;
                    let c2 = a + GRID as u16;
                    let d = c2 + 1;
                    indices.extend_from_slice(&[a, b, c2, b, d, c2]);
                }
            }
            out.push(FxPass::Mesh { verts, indices, modulate2x: false });
        }
        out
    }
}
