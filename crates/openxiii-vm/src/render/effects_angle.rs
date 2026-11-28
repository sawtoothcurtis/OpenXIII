//! Fix that is not in the original game (improved build only, never in the faithful build): smoke,
//! particles and halos that disappear depending on the view angle. In XIII, visibility
//! judges an Emitter by the box of its UPrimitiveNS: its origin ± (CollisionRadius
//! + 1, CollisionHeight + 1), a few dozen units, whatever the extent of its particles. A sprite
//! (DT_Sprite) is judged by its collision cylinder, much smaller than its 128 × DrawScale quad. As
//! soon as this small volume leaves the view, or its BSP leaves are no longer seen, the actor is
//! no longer rendered: the halo disappears at once, the smoke is no longer drawn by the game and
//! its emitters become Inactive (no more updates after SecondsBeforeInactive). Here, for
//! visibility only, an Emitter is judged by the box of its living particles (size included) and a
//! sprite by that of its quad; the BSP leaves are those of this box, rebuilt every frame. Both are
//! stored with the movable actors for this. Actor.Leaves, GetBoundingBox and collisions do not
//! change.

use crate::flags_classes::D;
use crate::value::ObjId;
use crate::NativeField as Field;
use crate::Vm;

type V = [f32; 3];
type Bx = (V, V);

/// DT_Sprite.
const DT_SPRITE: u8 = 1;
/// Margin of the quad of a particle or a sprite turned to face the camera (√2 rounded up).
const MARGIN_QUAD: f32 = 1.5;

/// Is the fix active? Faithful build: never.
pub fn active_value(graphics: crate::video_settings::SessionGraphics) -> bool {
    crate::improved::visual_effects::angle_effects(graphics)
}

thread_local! {
    static DBG_SEEN: std::cell::RefCell<std::collections::HashSet<ObjId>> = Default::default();
}

fn union(a: Bx, b: Bx) -> Bx {
    ([a.0[0].min(b.0[0]), a.0[1].min(b.0[1]), a.0[2].min(b.0[2])], [a.1[0].max(b.1[0]), a.1[1].max(b.1[1]), a.1[2].max(b.1[2])])
}

fn around(c: V, r: f32) -> Bx {
    ([c[0] - r, c[1] - r, c[2] - r], [c[0] + r, c[1] + r, c[2] + r])
}

impl Vm {
    /// Affected actor: an Emitter (except BeamEmitter, whose beams are drawn separately) or a
    /// sprite.
    pub(crate) fn effect_angle(&self, a: ObjId) -> bool {
        (self.is(a, D::Emitter) && !self.is(a, D::BeamEmitter)) || self.native_prop(a, Field::DrawType).as_byte() == DT_SPRITE
    }

    /// Corrected visibility box of `a` (always contains the game's one).
    pub(crate) fn effect_box(&mut self, a: ObjId) -> Bx {
        let base = self.render_bounding_box(a, false);
        let loc = self.native_prop(a, Field::Location).as_vector();
        if self.native_prop(a, Field::DrawType).as_byte() == DT_SPRITE && !self.is(a, D::Emitter) {
            let s = self.native_prop(a, Field::DrawScale).as_float().abs();
            let s3 = self.native_prop(a, Field::DrawScale3D).as_vector();
            let r = 64.0 * s * s3[0].abs().max(s3[1].abs()) * MARGIN_QUAD;
            return if r.is_finite() { union(base, around(loc, r)) } else { base };
        }
        let rot = self.native_prop(a, Field::Rotation).as_rotator();
        let mut out = base;
        for e in self.emitter_list(a).into_iter().flatten() {
            let local = self.propr(e, "CoordinateSystem").as_byte() == 1;
            let Some(state) = self.particles.states.get(&e) else { continue };
            if state.disabled && state.last_draw.is_none() {
                continue;
            }
            let ext = state.mesh_extent.iter().fold(1.0f32, |m, &x| m.max(x.abs()));
            let m = local.then(|| openxiii_assets::coords::rotation_translation_matrix(rot, loc));
            let n = state.active_particles.max(0) as usize;
            for q in state.particles.iter().take(n).filter(|q| q.flags & 1 != 0) {
                let p = match &m {
                    Some(m) => openxiii_assets::coords::transform_fvector(m, q.location),
                    None => q.location,
                };
                let r = q.size.iter().fold(0.0f32, |m, &x| m.max(x.abs())) * ext * MARGIN_QUAD;
                if p.iter().all(|c| c.is_finite()) && r.is_finite() {
                    out = union(out, around(p, r));
                }
            }
        }
        out
    }

    /// Before each scene: corrected boxes and leaves of the affected movable actors.
    pub(crate) fn effects_angle_prepare(&mut self) {
        self.display.effects.clear();
        if !active_value(self.engine.client.session_graphics) {
            return;
        }
        for i in self.display.num_static..self.display.entries.len() {
            let Some(a) = self.display.entries[i].actor else { continue };
            if !self.alive(a) || !self.effect_angle(a) {
                continue;
            }
            let (lo, hi) = self.effect_box(a);
            let Some(model) = self.level_bsp.as_ref() else { return };
            let mut leaves = model.box_leaves(lo, hi);
            if self.pb(a, "bVisibleOnlyOwnZone") {
                let zone = self.region_zone_number(a);
                let zones = &self.level_bsp.as_ref().unwrap().leaf_zones;
                leaves.retain(|&l| zones.get(l as usize).copied() == Some(zone));
            }
            self.display.effects.insert(a, ((lo, hi), leaves));
        }
    }
}

/// Halo controller (STY_Translucent sprites, XIII's fake "flares"): instead of appearing and
/// disappearing at once, each halo fades in when its center is in view (no BSP wall between
/// the camera and it), fades out when it goes behind a wall or leaves the view, and shines
/// more at the center of the view. Always drawn with the game's depth test.
pub mod halos {
    use super::*;

    /// Fade-in and fade-out speed, per second.
    const RISE: f32 = 5.0;
    const DESCENT: f32 = 3.0;
    /// Intensity at the edge of the view (1 at the center).
    const EDGE: f32 = 0.5;

    pub fn active_value(graphics: crate::video_settings::SessionGraphics) -> bool {
        super::active_value(graphics)
    }

    fn sub(a: V, b: V) -> V {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    fn dot(a: V, b: V) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    impl Vm {
        /// Is halo `a` in a sky zone (seen by the sky camera, not ours)?
        fn halo_in_sky(&self, a: ObjId) -> bool {
            let z = self.region_zone_number(a);
            self.zone_actor(z).is_some_and(|z| self.is_a(z, "SkyZoneInfo"))
        }

        /// Intensity (0 to 1) of halo `a` seen from `camera` looking towards `forward`, fade updated.
        /// `None`: not a managed halo (the sprite is drawn as in the game).
        pub fn halo_intensity(&mut self, a: ObjId, camera: V, forward: V) -> Option<f32> {
            if !active_value(self.engine.client.session_graphics) || self.halo_in_sky(a) {
                return None;
            }
            let l = self.native_prop(a, Field::Location).as_vector();
            let d = sub(l, camera);
            let dist = dot(d, d).sqrt();
            if !(dist > 1.0) {
                return None;
            }
            let dir = d.map(|c| c / dist);
            let cos = dot(dir, forward);
            // Center moved towards the camera: a halo stuck to or half inside a wall stays visible.
            let s = self.native_prop(a, Field::DrawScale).as_float().abs();
            let s3 = self.native_prop(a, Field::DrawScale3D).as_vector();
            let radius_list = 64.0 * s * s3[0].abs().max(s3[1].abs());
            let recoil = (radius_list * 0.25).clamp(8.0, 48.0).min(dist * 0.5);
            let target_value = [l[0] - dir[0] * recoil, l[1] - dir[1] * recoil, l[2] - dir[2] * recoil];
            let free = cos > 0.0 && self.level_bsp.as_ref().is_none_or(|m| m.fast_line_check(target_value, camera));
            let but = if free { 1.0 } else { 0.0 };
            let now = self.level_time;
            let view = if self.local_split() { self.current_viewport } else { None };
            let fade = {
                let e = self.display.halo_fades.entry((a, view)).or_insert((but, now));
                let dt = (now - e.1) as f32;
                e.0 = if !(0.0..=0.5).contains(&dt) {
                    but
                } else if but > e.0 {
                    (e.0 + RISE * dt).min(but)
                } else {
                    (e.0 - DESCENT * dt).max(but)
                };
                e.1 = now;
                e.0
            };
            let k = fade * (EDGE + (1.0 - EDGE) * smooth(0.64, 0.97, cos));
            Some(k)
        }
    }
}
