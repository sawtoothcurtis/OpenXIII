//! Trails (ATrail: bullets, rockets, grenades, knives), reimplemented from Engine.dll: execInit,
//! execReset, execAddSection, AddSection,
//! Tick and Destroy. The trail keeps a vertex buffer
//! (FTrailVertexStream, + 0x224): MaxSections sections of two vertices (twice as many in CrossMode,
//! the second ribbon following), of which Count sections are alive from Start. Each Tick
//! ages the vertices' alpha (255 / FadePeriod per second), brings together or spreads the two edges
//! of each section (ScaleLin), removes the faded sections and adds sections at the Owner's
//! position (SpawnFreq per second). Rendering (ATrail::Render) reads `trails`.
//! Floating-point operations in machine-code order (24-bit FPU).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::physics::V;
use crate::radius_iter::{local_to_world, transform_fvector};
use crate::value::ObjId;
use crate::Vm;

/// Vertex of a trail (0x18 bytes): position, colour (alpha in the high byte), U, V.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrailVertex {
    pub pos: V,
    pub color: u32,
    pub u: f32,
    pub v: f32,
}

/// Native data of a trail (PrivateData, + 0x224 to + 0x247).
#[derive(Debug, Clone, Default)]
pub struct TrailState {
    /// Vertices: section i = (verts[2i], verts[2i + 1]); in CrossMode the second ribbon starts at
    /// 2 × max.
    pub verts: Vec<TrailVertex>,
    /// + 0x228: number of sections in the buffer.
    pub max: u16,
    /// + 0x22A: first live section.
    pub start: u16,
    /// + 0x22C: number of live sections.
    pub count: u16,
    /// + 0x230: remainder of sections to create.
    spawn_acc: f32,
    /// + 0x234: remainder of alpha to remove.
    fade_acc: f32,
    /// + 0x238: current V coordinate.
    uv: f32,
    /// + 0x23C: last position added.
    last: V,
    /// Bit 0x10 of + 0x220: the next AddSection only sets the position.
    first: bool,
}

/// FVector::Size: (X² + Y²) + Z², appSqrt in double.
fn size(v: V) -> f32 {
    (((v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]) as f64).sqrt() as f32
}

/// FVector::Normalize: false below 1e-8.
fn normalize(v: &mut V) -> bool {
    let sq = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
    if (sq as f64) < 1e-8 {
        return false;
    }
    let r = 1.0 / sq.sqrt();
    *v = [r * v[0], r * v[1], r * v[2]];
    true
}

/// (_ftol) then FILD of the unsigned integer: integer part and remainder.
fn ftol_split(x: f32) -> (u32, f32) {
    let n = crate::value::ftol(x) as i32 as u32;
    (n, x - n as f32)
}

/// Trail fields read by ATrail::AddSection.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ParamsSection {
    pub location: V,
    pub collision_radius: f32,
    pub width: f32,
    pub uv_scale: f32,
    pub cur_rotation: f32,
    pub cross: bool,
    pub begin_point: bool,
}

/// ATrail::AddSection: section at `pos`, perpendicular to `dir` (normalized in
/// place), of half-width width (zero for the first point if BeginPoint), rotated by
/// CurRotation; if `pos` is further than CollisionRadius, `grows` receives 1.2 × the distance
/// (CollisionRadius and CollisionHeight, then FarMoveActor in place).
pub(crate) fn add_section_core(p: &mut ParamsSection, st: &mut TrailState, pos: V, dir: &mut V, mut grows: impl FnMut(f32)) {
    let l = p.location;
    let d = [l[0] - pos[0], l[1] - pos[1], l[2] - pos[2]];
    let dist = (((d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]) as f64).sqrt() as f32;
    if dist > p.collision_radius {
        // CollisionRadius and CollisionHeight are written into the actor:
        // the next call reads them back.
        p.collision_radius = dist * 1.2;
        grows(dist * 1.2);
    }
    let len = (((dir[0] * dir[0] + dir[1] * dir[1]) + dir[2] * dir[2]) as f64).sqrt() as f32;
    if len == 0.0 {
        return;
    }
    let inv = 1.0 / len;
    *dir = [inv * dir[0], inv * dir[1], inv * dir[2]];
    let dd = *dir;
    // R = Dir ^ (0, 0, 1), otherwise Dir ^ (0,1, 0, 1) (FMUL by the machine code's constants).
    let z0 = dd[2] * 0.0;
    let mut r = [dd[1] - z0, z0 - dd[0], dd[0] * 0.0 - dd[1] * 0.0];
    if !normalize(&mut r) {
        let z1 = dd[2] * 0.1;
        r = [dd[1] - z1, z1 - dd[0], dd[0] * 0.1 - dd[1] * 0.1];
        normalize(&mut r);
    }
    let mut u = [r[1] * dd[2] - r[2] * dd[1], r[2] * dd[0] - r[0] * dd[2], r[0] * dd[1] - r[1] * dd[0]];
    if p.begin_point && st.count == 0 {
        r = r.map(|x| x * 0.0);
        u = u.map(|x| x * 0.0);
    } else {
        r = r.map(|x| x * p.width);
        u = u.map(|x| x * p.width);
    }
    let rot = p.cur_rotation;
    if rot != 0.0 {
        // appSin stored as a float; appCos stays in st(0): products by cos in a
        // single rounding.
        let s = (rot as f64).dsin() as f32;
        let c = (rot as f64).dcos();
        let fc = |x: f32| (x as f64 * c) as f32;
        let nu = [0, 1, 2].map(|k| fc(u[k]) - r[k] * s);
        let nr = [0, 1, 2].map(|k| u[k] * s + fc(r[k]));
        u = nu;
        r = nr;
    }
    let (start, count, max, uv0) = (st.start, st.count, st.max, st.uv);
    let i = (start as usize + count as usize) * 2;
    let green = |p: V, off: V, sign: f32, uu: f32| TrailVertex {
        pos: if sign > 0.0 { [off[0] + p[0], off[1] + p[1], off[2] + p[2]] } else { [p[0] - off[0], p[1] - off[1], p[2] - off[2]] },
        color: 0xff00_0000,
        u: uu,
        v: uv0,
    };
    if let Some(s) = st.verts.get_mut(i..i + 2) {
        s[0] = green(pos, u, 1.0, 0.0);
        s[1] = green(pos, u, -1.0, 1.0);
    }
    if p.cross {
        let j = i + max as usize * 2;
        if let Some(s) = st.verts.get_mut(j..j + 2) {
            s[0] = green(pos, r, 1.0, 0.0);
            s[1] = green(pos, r, -1.0, 1.0);
        }
    }
    st.last = pos;
    st.uv += len * p.uv_scale;
    st.count += 1;
    st.first = false;
}

/// Trail settings read by ATrail::Tick.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ParamsTick {
    pub auto_destroy: bool,
    pub fade_period: f32,
    pub scale_lin: f32,
    pub cross: bool,
    pub rotation_speed: f32,
    pub spawn_freq: f32,
    /// The Owner (non-null) has bDeleteMe: Owner is set to null.
    pub owner_deleted: bool,
}

/// ATrail::Tick, after AActor::Tick. `owner` is the Owner's position (Owner non-null),
/// reset to None if the Owner is destroyed. Returns true if the trail must be destroyed
/// (DestroyActor): AutoDestroy without an Owner or a section, or buffer missing.
pub(crate) fn tick_core(
    t: &ParamsTick,
    sec: &mut ParamsSection,
    st: Option<&mut TrailState>,
    dt: f32,
    owner: &mut Option<V>,
    mut grows: impl FnMut(f32),
) -> bool {
    if owner.is_some() && t.owner_deleted {
        *owner = None;
    }
    let empty = st.as_ref().is_none_or(|s| s.count == 0);
    let Some(st) = st else { return true };
    if t.auto_destroy && owner.is_none() && empty {
        return true;
    }
    let (fade, scale_lin, cross, rotation_speed, spawn_freq) = (t.fade_period, t.scale_lin, t.cross, t.rotation_speed, t.spawn_freq);
    let (fade_n, rest) = ftol_split((dt / fade) * 255.0 + st.fade_acc);
    st.fade_acc = rest;
    let scale = dt * scale_lin;
    let second = st.max as usize * 2;
    let mut dead = 0u32;
    for k in 0..st.count as usize {
        let i = (st.start as usize + k) * 2;
        if i + 1 >= st.verts.len() {
            break;
        }
        let alpha = st.verts[i].color >> 24;
        let left = if alpha > fade_n {
            alpha - fade_n
        } else {
            dead += 1;
            0
        };
        let color = left << 24;
        st.verts[i].color = color;
        st.verts[i + 1].color = color;
        let (pa, pb) = (st.verts[i].pos, st.verts[i + 1].pos);
        let mut d = [pa[0] - pb[0], pa[1] - pb[1], pa[2] - pb[2]];
        if scale > 0.0 {
            normalize(&mut d);
            let m = d.map(|x| x * scale);
            st.verts[i].pos = [m[0] + pa[0], m[1] + pa[1], m[2] + pa[2]];
            st.verts[i + 1].pos = [pb[0] - m[0], pb[1] - m[1], pb[2] - m[2]];
        } else if scale < 0.0 {
            let len = size(d);
            if -0.5 * len < scale {
                let k = scale / len;
                let m = d.map(|x| x * k);
                st.verts[i].pos = [m[0] + pa[0], m[1] + pa[1], m[2] + pa[2]];
                st.verts[i + 1].pos = [pb[0] - m[0], pb[1] - m[1], pb[2] - m[2]];
            } else {
                // The two edges meet in the middle.
                let mid = [(pa[0] + pb[0]) * 0.5, (pb[1] + pa[1]) * 0.5, (pb[2] + pa[2]) * 0.5];
                st.verts[i].pos = mid;
                st.verts[i + 1].pos = mid;
            }
        }
        if cross && i + second + 1 < st.verts.len() {
            let j = i + second;
            st.verts[j].color = color;
            st.verts[j + 1].color = color;
            let (qa, qb) = (st.verts[j].pos, st.verts[j + 1].pos);
            let mut e = [qa[0] - qb[0], qa[1] - qb[1], qa[2] - qb[2]];
            normalize(&mut e);
            let s2 = dt * scale_lin;
            let m = e.map(|x| x * s2);
            st.verts[j].pos = [m[0] + qa[0], m[1] + qa[1], m[2] + qa[2]];
            st.verts[j + 1].pos = [qb[0] - m[0], qb[1] - m[1], qb[2] - m[2]];
        }
    }
    // Faded sections removed (all but one), buffer brought back to the start if it overflows.
    if dead > 1 {
        let n = (dead - 1) as u16;
        st.start = st.start.wrapping_add(n);
        st.count = st.count.wrapping_sub(n);
        if st.start as u32 + st.count as u32 >= st.max as u32 {
            let (s, c) = (st.start as usize * 2, st.count as usize * 2);
            st.verts.copy_within(s..(s + c).min(second), 0);
            if cross {
                let n = st.verts.len();
                st.verts.copy_within(second + s..(second + s + c).min(n), second);
            }
            st.start = 0;
        }
    }
    let rot = if rotation_speed != 0.0 { Some(dt * rotation_speed * 6.2831855) } else { None };
    let (spawn_n, rest) = ftol_split(dt * spawn_freq + st.spawn_acc);
    st.spawn_acc = rest;
    let (count, start, max, last) = (st.count, st.start, st.max, st.last);
    if let Some(r) = rot {
        sec.cur_rotation = r + sec.cur_rotation;
    }
    let Some(pos) = *owner else { return false };
    if spawn_n == 0 || start as u32 + count as u32 >= max as u32 {
        return false;
    }
    let mut dir = [last[0] - pos[0], last[1] - pos[1], last[2] - pos[2]];
    if count == 0 && (start as u32 + 1) < max as u32 && size(dir) > 0.0 {
        add_section_core(sec, st, last, &mut dir, &mut grows);
    }
    add_section_core(sec, st, pos, &mut dir, &mut grows);
    false
}

impl Vm {
    /// Transformed Owner position: Owner.LocalToWorld × ActorOffset.
    fn trail_owner_point(&self, owner: ObjId, a: ObjId) -> V {
        let m = local_to_world(
            self.propr(owner, "Rotation").as_rotator(),
            self.native_prop(owner, crate::NativeField::DrawScale3D).as_vector(),
            self.native_prop(owner, crate::NativeField::Location).as_vector(),
            self.native_prop(owner, crate::NativeField::PrePivot).as_vector(),
            self.native_prop(owner, crate::NativeField::DrawScale).as_float(),
        );
        transform_fvector(&m, self.pv(a, "ActorOffset"))
    }

    /// ATrail::execInit: new buffer of ftol(FadePeriod × SpawnFreq + 10)
    /// sections; on the Owner, zero Rotation and FarMoveActor(bNoCheck).
    pub(crate) fn trail_init(&mut self, a: ObjId) {
        let max = ftol_split(self.pf(a, "FadePeriod") * self.pf(a, "SpawnFreq") + 10.0).0 as u16;
        let cross = self.pb(a, "CrossMode");
        let n = if cross { max as usize * 4 } else { max as usize * 2 };
        let mut st = TrailState { verts: vec![TrailVertex::default(); n], max, spawn_acc: 1.0, first: true, ..Default::default() };
        if let Some(owner) = self.native_object(a, crate::NativeField::Owner) {
            st.last = self.trail_owner_point(owner, a);
            let last = st.last;
            self.trails.insert(a, st);
            self.set_prop(a, "Rotation", crate::value::Value::rotator(0, 0, 0));
            self.far_move_actor(a, last, false, true, false);
        } else {
            self.trails.insert(a, st);
        }
    }

    /// ATrail::execReset: trail emptied, collision 1 × 1, then FarMoveActor onto
    /// the Owner (bNoCheck) or in place.
    pub(crate) fn trail_reset(&mut self, a: ObjId) {
        self.set_f(a, "CollisionRadius", 1.0);
        self.set_f(a, "CollisionHeight", 1.0);
        let owner = self.native_object(a, crate::NativeField::Owner);
        let last = owner.map(|o| self.trail_owner_point(o, a));
        if let Some(st) = self.trails.get_mut(&a) {
            st.spawn_acc = 1.0;
            st.start = 0;
            st.count = 0;
            st.fade_acc = 0.0;
            st.uv = 0.0;
            st.first = true;
            if let Some(l) = last {
                st.last = l;
            }
        }
        match last {
            Some(l) => {
                self.set_prop(a, "Rotation", crate::value::Value::rotator(0, 0, 0));
                self.far_move_actor(a, l, false, true, false);
            }
            None => {
                let l = self.native_prop(a, crate::NativeField::Location).as_vector();
                self.far_move_actor(a, l, false, false, false);
            }
        }
    }

    /// ATrail::execAddSection: the first time, only sets the position; otherwise
    /// a joining section (empty trail) then the section at `pos`.
    pub(crate) fn trail_add_section_native(&mut self, a: ObjId, pos: V) {
        let Some(st) = self.trails.get_mut(&a) else { return };
        if st.first {
            st.last = pos;
            st.first = false;
            return;
        }
        let last = st.last;
        let mut dir = [last[0] - pos[0], last[1] - pos[1], last[2] - pos[2]];
        if st.count == 0 && (st.start as u32 + 1) < st.max as u32 && size(dir) > 0.0 {
            self.trail_add_section(a, last, &mut dir);
        }
        self.trail_add_section(a, pos, &mut dir);
    }

    /// ATrail::AddSection, see `add_section_core`.
    fn trail_add_section(&mut self, a: ObjId, pos: V, dir: &mut V) {
        let l = self.native_prop(a, crate::NativeField::Location).as_vector();
        let mut p = ParamsSection {
            location: l,
            collision_radius: self.native_prop(a, crate::NativeField::CollisionRadius).as_float(),
            width: self.pf(a, "width"),
            uv_scale: self.pf(a, "UVScale"),
            cur_rotation: self.pf(a, "CurRotation"),
            cross: self.pb(a, "CrossMode"),
            begin_point: self.pb(a, "BeginPoint"),
        };
        let Some(mut st) = self.trails.remove(&a) else { return };
        add_section_core(&mut p, &mut st, pos, dir, |r| {
            self.set_f(a, "CollisionRadius", r);
            self.set_f(a, "CollisionHeight", r);
            self.far_move_actor(a, l, false, false, false);
        });
        self.trails.insert(a, st);
    }

    /// ATrail::Tick, after AActor::Tick; see `tick_core`.
    pub(crate) fn trail_tick(&mut self, a: ObjId, dt: f32) {
        let owner = self.native_object(a, crate::NativeField::Owner);
        let owner_deleted = owner.is_some_and(|o| self.native_prop(o, crate::NativeField::DeleteMe).as_bool());
        let t = ParamsTick {
            auto_destroy: self.pb(a, "AutoDestroy"),
            fade_period: self.pf(a, "FadePeriod"),
            scale_lin: self.pf(a, "ScaleLin"),
            cross: self.pb(a, "CrossMode"),
            rotation_speed: self.pf(a, "RotationSpeed"),
            spawn_freq: self.pf(a, "SpawnFreq"),
            owner_deleted,
        };
        let l = self.native_prop(a, crate::NativeField::Location).as_vector();
        let mut sec = ParamsSection {
            location: l,
            collision_radius: self.native_prop(a, crate::NativeField::CollisionRadius).as_float(),
            width: self.pf(a, "width"),
            uv_scale: self.pf(a, "UVScale"),
            cur_rotation: self.pf(a, "CurRotation"),
            cross: t.cross,
            begin_point: self.pb(a, "BeginPoint"),
        };
        let mut point = owner.map(|o| self.trail_owner_point(o, a));
        let Some(mut st) = self.trails.remove(&a) else {
            let _ = self.destroy_actor(a);
            return;
        };
        let cur_before = sec.cur_rotation;
        let destroyed = tick_core(&t, &mut sec, Some(&mut st), dt, &mut point, |r| {
            self.set_f(a, "CollisionRadius", r);
            self.set_f(a, "CollisionHeight", r);
            self.far_move_actor(a, l, false, false, false);
        });
        if owner.is_some() && point.is_none() {
            self.set_prop(a, "Owner", crate::value::Value::Object(None));
        }
        if sec.cur_rotation.to_bits() != cur_before.to_bits() {
            self.set_f(a, "CurRotation", sec.cur_rotation);
        }
        if destroyed {
            self.trails.insert(a, st);
            let _ = self.destroy_actor(a);
            return;
        }
        self.trails.insert(a, st);
    }
}

