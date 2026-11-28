//! Weather (snow, rain): "RndCubeSpr" of FD3DRenderInterface (D3DDrv.dll), set by the
//! LevelInfo natives (vtables +0x70 to +0x84) and animated by UGameEngine::Tick (vtable +0x88), drawn by (vtable +0x8C, DrawRndCubeSpr).
//!
//! A cube of 256 units contains points (at most MaxNbrSpr / 8); it is repeated in
//! each 256 cell of the world that falls in the view pyramid up to Distance, except in the
//! exclusion boxes, and drawn as Direct3D point sprites (size height × Size /
//! distance, at most SizeMax). The wind (gusts) and each point's jitter advance with
//! appFrand, shared with the script.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::Value;
use crate::Vm;

type V = [f32; 3];

/// Side of the cube and its half.
const CUBE: f32 = 256.0;
const HALF: f32 = 128.0;
/// D3DCAPS8::MaxPointSize of a current graphics card (8192), the value used for the reference captures: Direct3D
/// clamps the point size to min(D3DRS_POINTSIZE_MAX, MaxPointSize).
pub const MAX_POINT_SIZE: f32 = 8192.0;

/// Point of the cube (0x1C-byte entry of the table at +0xDF0): position, D3DCOLOR colour,
/// jitter.
#[derive(Debug, Clone, Copy, Default)]
pub struct Flake {
    pub pos: V,
    pub color: u32,
    pub wobble: V,
}

/// Exclusion box (0x34 bytes at +0xDD4): bounds rounded to 256, centre, half-sides, radius.
#[derive(Debug, Clone, Copy, Default)]
struct Exclude {
    min: V,
    max: V,
    center: V,
    half: V,
    radius: f32,
}

/// State of FD3DRenderInterface (+0xD9C … +0xDF4) and D3DDrv's static variables.
#[derive(Debug, Clone)]
pub struct RndCubeSpr {
    /// +0xD9C: material (the points' texture).
    pub material: Option<String>,
    /// +0xDA0: active; +0xDA4: mask (alpha test) instead of additive blending.
    pub enabled: bool,
    pub mask: bool,
    /// +0xDA8: number of points of the cube; +0xDAC: visible points; +0xDB0: full points.
    count: i32,
    pub active: i32,
    full: i32,
    /// +0xDB4: distance; +0xDB8: speed; +0xDC4: jitter; +0xDC8: acceleration.
    distance: f32,
    speed: V,
    random_speed: f32,
    random_acc: f32,
    /// +0xDCC / +0xDD0: size and maximum size of the sprites.
    pub size: f32,
    pub size_max: f32,
    pub flakes: Vec<Flake>,
    excludes: Vec<Exclude>,
    /// D0: direction of the gusts; C4: wind.
    gust_dir: Option<V>,
    wind: Option<V>,
    /// Time and period of the gusts.
    gust_time: f32,
    gust_period: f32,
    /// Time and period of the wind strength;: target
    /// strength;: strength.
    force_time: f32,
    force_period: f32,
    force_target: f32,
    force: f32,
    /// Remainder of the number of points that change per call.
    change_rest: f32,
}

impl Default for RndCubeSpr {
    fn default() -> Self {
        RndCubeSpr {
            material: None,
            enabled: false,
            mask: false,
            count: 0,
            active: 0,
            full: 0,
            distance: 0.0,
            speed: [0.0; 3],
            random_speed: 0.0,
            random_acc: 0.0,
            size: 0.0,
            size_max: 0.0,
            flakes: Vec::new(),
            excludes: Vec::new(),
            gust_dir: None,
            wind: None,
            gust_time: 0.0,
            gust_period: 1.0,
            force_time: 0.0,
            force_period: 1.0,
            force_target: 0.0,
            force: 0.0,
            change_rest: 0.0,
        }
    }
}

impl RndCubeSpr {
    /// D3DDrv's static variables (gusts, wind, strength: D4, remainder
    /// of the changes): read and written only by AnimRndCubeSpr and ChangeRndCubeSprProp, never when a map is
    /// loaded: they survive travel and reloads.
    pub(crate) fn kept_static(&mut self, prior: &RndCubeSpr) {
        self.gust_dir = prior.gust_dir;
        self.wind = prior.wind;
        self.gust_time = prior.gust_time;
        self.gust_period = prior.gust_period;
        self.force_time = prior.force_time;
        self.force_period = prior.force_period;
        self.force_target = prior.force_target;
        self.force = prior.force;
        self.change_rest = prior.change_rest;
    }
}

impl RndCubeSpr {
    pub fn static_text(&self) -> String {
        let v = |x: Option<V>| x.map_or("-".to_string(), |v| v.iter().map(|f| format!("{:08x}", f.to_bits())).collect::<Vec<_>>().join(","));
        let f = |x: f32| format!("{:08x}", x.to_bits());
        format!(
            "{};{};{};{};{};{};{};{};{}",
            v(self.gust_dir),
            v(self.wind),
            f(self.gust_time),
            f(self.gust_period),
            f(self.force_time),
            f(self.force_period),
            f(self.force_target),
            f(self.force),
            f(self.change_rest)
        )
    }

    /// Reads `static_text` back; nothing is changed if the text is malformed.
    pub fn static_place_text(&mut self, s: &str) -> bool {
        let c: Vec<&str> = s.trim().split(';').collect();
        let f = |x: &str| u32::from_str_radix(x.trim(), 16).ok().map(f32::from_bits);
        let v = |x: &str| -> Option<Option<V>> {
            if x.trim() == "-" {
                return Some(None);
            }
            let p: Vec<f32> = x.split(',').map(f).collect::<Option<Vec<_>>>()?;
            (p.len() == 3).then(|| Some([p[0], p[1], p[2]]))
        };
        let (Some(g), Some(w)) = (c.first().and_then(|x| v(x)), c.get(1).and_then(|x| v(x))) else { return false };
        let n: Option<Vec<f32>> = c.iter().skip(2).map(|x| f(x)).collect();
        let Some(n) = n.filter(|n| n.len() == 7) else { return false };
        self.gust_dir = g;
        self.wind = w;
        (self.gust_time, self.gust_period, self.force_time, self.force_period, self.force_target, self.force, self.change_rest) = (n[0], n[1], n[2], n[3], n[4], n[5], n[6]);
        true
    }
}

/// What DrawRndCubeSpr draws: the visible points, repeated at the origin of each cell.
#[derive(Debug, Clone, Default)]
pub struct WeatherDraw {
    pub texture: String,
    pub mask: bool,
    pub size: f32,
    /// D3DRS_POINTSIZE_MAX (+0xDD0) clamped by MaxPointSize.
    pub size_max: f32,
    /// Corners of the drawn 256 cells (SetTransform(WORLD) then DrawPrimitive(POINTLIST)).
    pub cells: Vec<V>,
    /// The first `active` points: position in the cube, D3DCOLOR.
    pub flakes: Vec<(V, u32)>,
}

/// Grey (v, v, v) with zero alpha (FColor((v << 16) | (v << 8) | v)).
fn gray(v: i32) -> u32 {
    let v = v as u32 & 0xff;
    (v << 16) | (v << 8) | v
}

/// fistp (round to nearest, ties to even).
fn round_even(x: f32) -> i32 {
    x.round_ties_even() as i32
}

/// FVector::Normalize; false if the squared length is below 1e-8.
fn normalize(v: &mut V) -> bool {
    let s = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if (s as f64) < 1e-8 {
        return false;
    }
    let r = crate::natives::app_rsqrt(s);
    *v = [r * v[0], r * v[1], r * v[2]];
    true
}

/// CameraToWorld of a view (FCameraSceneNode: inverse of T(−origin) × inverse rotation ×
/// screen frame): rows right, down, forward, origin. Computed directly from
/// FRotationTranslationMatrix instead of FMatrix::Inverse.
pub fn camera_to_world(origin: V, rotation: [i32; 3]) -> [[f32; 4]; 4] {
    let m = openxiii_assets::coords::rotation_translation_matrix(rotation, origin);
    [[m[4], m[5], m[6], 0.0], [-m[8], -m[9], -m[10], 0.0], [m[0], m[1], m[2], 0.0], [origin[0], origin[1], origin[2], 1.0]]
}

impl Vm {
    /// Points drawn as do: three appFrand × 256 − 128, stored as
    /// (third, second, first).
    fn flake_position(&mut self) -> V {
        let a = self.rand() * CUBE - HALF;
        let b = self.rand() * CUBE - HALF;
        let c = self.rand() * CUBE - HALF;
        [c, b, a]
    }

    /// Vtable +0x70: ALevelInfo::execInitRndCubeSpr.
    pub(crate) fn rnd_cube_init(&mut self, texture: Option<String>, max: i32, prop: f32, distance: f32) {
        let w = &mut self.weather;
        w.excludes.clear();
        let count = max / 8;
        w.material = texture;
        if count != w.count {
            w.count = count;
            w.flakes = vec![Flake::default(); count.max(0) as usize];
        }
        let count = w.count;
        w.active = ((count as f32 * prop) as i32).clamp(0, count.max(0)).min(count);
        w.full = w.active;
        w.speed = [10.0, 10.0, -50.0];
        w.distance = distance;
        w.random_speed = 0.0;
        w.random_acc = 0.0;
        w.size = 5.0;
        w.size_max = 64.0;
        w.enabled = false;
        if w.material.is_none() || count <= 0 {
            return;
        }
        for i in 0..self.weather.active as usize {
            let pos = self.flake_position();
            self.weather.flakes[i] = Flake { pos, color: 0xffff_ffff, wobble: [0.0; 3] };
        }
    }

    /// UD3DRenderDevice::SetRes recreates the points' vertex buffer and their table
    /// if there is a material and points, the visible
    /// points are drawn again as at initialization (three appFrand each, white, without
    /// jitter); the rest of the state (speed, size, exclusion boxes) does not move.
    pub(crate) fn rnd_cube_set_res(&mut self) {
        // a new vertex buffer and a new point table,
        // zeroed (rep stosd), as soon as there are points; the invisible points are zero there.
        if self.weather.count > 0 {
            self.weather.flakes = vec![Flake::default(); self.weather.count as usize];
        }
        if self.weather.material.is_none() || self.weather.count <= 0 {
            return;
        }
        for i in 0..self.weather.active.max(0) as usize {
            let pos = self.flake_position();
            self.weather.flakes[i] = Flake { pos, color: 0xffff_ffff, wobble: [0.0; 3] };
        }
    }

    /// Vtable +0x80: execSetRndCubeSprState.
    pub(crate) fn rnd_cube_state(&mut self, on: bool) {
        let w = &mut self.weather;
        if on {
            if !w.enabled && w.count >= 1 && w.material.is_some() {
                w.enabled = true;
            }
        } else {
            w.enabled = false;
        }
    }

    /// Vtable +0x74: execSetRndCubeSprSpeed.
    pub(crate) fn rnd_cube_speed(&mut self, speed: V, random_speed: f32, random_acc: f32) {
        let w = &mut self.weather;
        w.speed = speed;
        w.random_speed = random_speed;
        w.random_acc = random_acc;
    }

    /// Vtable +0x84: execSetRndCubeSprSize (SizeMax 64 and IsMask
    /// false by default).
    pub(crate) fn rnd_cube_size(&mut self, size: f32, size_max: f32, mask: bool) {
        let w = &mut self.weather;
        w.size = size;
        w.size_max = size_max;
        w.mask = mask;
    }

    /// Vtable +0x7C: execAddRndCubeSprExclude. Bounds rounded
    /// to the 256 cells (fistp of p / 256 − 0.5; of (p + 255) / 256 − 0.5 for the maximum).
    pub(crate) fn rnd_cube_exclude(&mut self, min: V, max: V) {
        let lo = min.map(|p| round_even(p * 0.003_906_25 - 0.5) as f32 * CUBE);
        let hi = max.map(|p| round_even((p + 255.0) * 0.003_906_25 - 0.5) as f32 * CUBE);
        let half = [(hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5, (hi[2] - lo[2]) * 0.5];
        let center = [half[0] + lo[0], half[1] + lo[1], half[2] + lo[2]];
        // (X² + Y²) + Z², root in double.
        let radius = ((half[0] * half[0] + half[1] * half[1] + half[2] * half[2]) as f64).sqrt() as f32;
        self.weather.excludes.push(Exclude { min: lo, max: hi, center, half, radius });
    }

    /// Vtable +0x78: execChangeRndCubeSprProp. Makes at most
    /// NbrSprFadePerLoop points (cumulative) appear or fade towards the desired proportion, each one
    /// by FadeSpeed × 256 grey levels per call; true once the proportion is reached.
    pub(crate) fn rnd_cube_change(&mut self, proportion: f32, fade_speed: f32, per_loop: f32) -> bool {
        let count = self.weather.count;
        let target = ((count as f32 * proportion) as i32).max(0).min(count);
        let w = &self.weather;
        if w.active == w.full && w.active == target {
            return true;
        }
        let step = ((fade_speed * CUBE) as i32).max(1);
        if !(per_loop > 0.0) {
            return false;
        }
        let acc = self.weather.change_rest + per_loop;
        let mut budget = acc as i32;
        self.weather.change_rest = acc - budget as f32;
        if target < self.weather.active {
            let mut i = self.weather.active - 1;
            while i >= target {
                let f = &mut self.weather.flakes[i as usize];
                let b = (f.color & 0xff) as i32;
                if b == 0xff {
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                    self.weather.full -= 1;
                    let v = 0xff - step;
                    if v > 0 {
                        self.weather.flakes[i as usize].color = gray(v);
                    } else {
                        self.weather.flakes[i as usize].color = 0;
                        self.weather.active -= 1;
                    }
                } else {
                    let v = b - step;
                    if v <= 0 {
                        f.color = 0;
                        self.weather.active -= 1;
                    } else {
                        f.color = gray(v);
                    }
                }
                i -= 1;
            }
        } else {
            let mut i = self.weather.full;
            while i < target {
                let b = (self.weather.flakes[i as usize].color & 0xff) as i32;
                if b == 0 {
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                    self.weather.active += 1;
                    let pos = self.flake_position();
                    let color = if step < 0xff {
                        gray(step)
                    } else {
                        self.weather.full += 1;
                        0x00ff_ffff
                    };
                    self.weather.flakes[i as usize] = Flake { pos, color, wobble: [0.0; 3] };
                } else {
                    let v = b + step;
                    let color = if v < 0xff {
                        gray(v)
                    } else {
                        self.weather.full += 1;
                        0x00ff_ffff
                    };
                    self.weather.flakes[i as usize].color = color;
                }
                i += 1;
            }
        }
        let w = &self.weather;
        w.active == w.full && w.active == target
    }

    /// Vtable +0x88 (AnimRndCubeSpr): gusts and wind strength drawn at random,
    /// then movement of the points (speed, wind, jitter) brought back into the cube.
    pub(crate) fn rnd_cube_anim(&mut self, dt: f32) {
        if !self.weather.enabled || self.weather.active == 0 {
            return;
        }
        let s = self.weather.speed;
        let ds = [s[0] * dt, s[1] * dt, s[2] * dt];
        if self.weather.gust_dir.is_none() {
            self.weather.gust_dir = Some([1.0, 0.0, 0.0]);
        }
        if self.weather.wind.is_none() {
            self.weather.wind = Some([0.0; 3]);
        }
        self.weather.gust_time += dt;
        self.weather.force_time += dt;
        if self.weather.gust_time > self.weather.gust_period {
            let mut dir = self.weather.gust_dir.unwrap();
            dir[2] = 0.0;
            loop {
                dir[0] = self.rand() - 0.5;
                dir[1] = self.rand() - 0.5;
                if normalize(&mut dir) {
                    break;
                }
            }
            self.weather.gust_dir = Some(dir);
            self.weather.gust_time = 0.0;
            self.weather.gust_period = (self.rand() + 1.0) * 10.0;
        }
        if self.weather.force_time > self.weather.force_period {
            // (1 − cos(appFrand × π/2)) × 5; fcos on the extended value.
            let a = self.rand() * std::f32::consts::FRAC_PI_2;
            self.weather.force_time = 0.0;
            self.weather.force_target = (((1.0 - (a as f64).dcos()) as f32 as f64) * 5.0) as f32;
            self.weather.force_period = self.rand() * 5.0 + 1.0;
        }
        let w = &mut self.weather;
        w.force = (w.force_target - w.force) * dt + w.force;
        let dir = w.gust_dir.unwrap();
        let mut wind = w.wind.unwrap();
        wind[0] = (w.force * dir[0] - wind[0]) * w.force * dt + wind[0];
        wind[1] = (w.force * dir[1] - wind[1]) * w.force * dt + wind[1];
        w.wind = Some(wind);
        let wrap = |p: &mut V| {
            for c in p.iter_mut() {
                if *c < 0.0 {
                    *c += CUBE;
                }
            }
            for c in p.iter_mut() {
                if CUBE <= *c {
                    *c -= CUBE;
                }
            }
        };
        let n = self.weather.active.max(0) as usize;
        if self.weather.random_acc == 0.0 {
            for f in self.weather.flakes.iter_mut().take(n) {
                f.pos = [ds[0] + f.pos[0], ds[1] + f.pos[1], ds[2] + f.pos[2]];
                wrap(&mut f.pos);
            }
            return;
        }
        for i in 0..n {
            let acc = self.weather.random_acc;
            let mut wo = self.weather.flakes[i].wobble;
            for c in wo.iter_mut() {
                *c = (self.rand() - 0.5) * acc + *c;
            }
            for c in wo.iter_mut() {
                if *c < -1.0 {
                    *c = -1.0;
                }
            }
            for c in wo.iter_mut() {
                if *c > 1.0 {
                    *c = 1.0;
                }
            }
            let w = &mut self.weather;
            let k = w.random_speed;
            let f = &mut w.flakes[i];
            f.wobble = wo;
            f.pos[0] = k * wo[0] * dt + f.pos[0] + wind[0] + ds[0];
            f.pos[1] = k * wo[1] * dt + f.pos[1] + wind[1] + ds[1];
            f.pos[2] = dt * k * wo[2] + f.pos[2] + wind[2] + ds[2];
            wrap(&mut f.pos);
        }
    }

    /// Vtable +0x8C (DrawRndCubeSpr) with CameraToWorld (rows X right, Y down,
    /// Z forward, origin): 256 cells between the corners of the view at Distance (90° field over
    /// 0.75 in height), kept if their centre passes the five planes (the back plane at
    /// Distance), outside the nearby and visible exclusion boxes.
    pub fn weather_draw(&self, m: &[[f32; 4]; 4]) -> Option<WeatherDraw> {
        let w = &self.weather;
        if !w.enabled || w.active == 0 {
            return None;
        }
        let (x, y, z, o) = ([m[0][0], m[0][1], m[0][2]], [m[1][0], m[1][1], m[1][2]], [m[2][0], m[2][1], m[2][2]], [m[3][0], m[3][1], m[3][2]]);
        let d = w.distance;
        let vy = [y[0] * 0.75, y[1] * 0.75, y[2] * 0.75];
        let corner = |sx: f32, sy: f32| -> V {
            let mut c = [0.0; 3];
            for k in 0..3 {
                let a = if sx < 0.0 { z[k] - x[k] } else { z[k] + x[k] };
                let b = if sy < 0.0 { a - vy[k] } else { vy[k] + a };
                c[k] = d * b + o[k];
            }
            c
        };
        let mut lo = o;
        let mut hi = o;
        for c in [corner(-1.0, -1.0), corner(-1.0, 1.0), corner(1.0, -1.0), corner(1.0, 1.0)] {
            for k in 0..3 {
                if c[k] < lo[k] {
                    lo[k] = c[k];
                }
                if hi[k] < c[k] {
                    hi[k] = c[k];
                }
            }
        }
        let snap = |v: f32| ((v * 0.003_906_25) as f64).floor() as f32 * CUBE;
        lo = lo.map(snap);
        hi = hi.map(snap);
        // Planes: −Z, then (−Z − X), (X − Z), (−0.75 Z − Y), (Y − 0.75 Z) normalized (
        // the FLD after the appRsqrt call comes before the "add esp, 4": it reads X, not Y).
        let fix = |mut n: V| -> V {
            let s = n[2] * n[2] + n[1] * n[1] + n[0] * n[0];
            if !((s as f64) < 1e-8) {
                let r = crate::natives::app_rsqrt(s);
                n = [n[0] * r, n[1] * r, r * n[2]];
            }
            n
        };
        let planes: [V; 5] = [
            [-z[0], -z[1], -z[2]],
            fix([-z[0] - x[0], -z[1] - x[1], -z[2] - x[2]]),
            fix([x[0] - z[0], x[1] - z[1], x[2] - z[2]]),
            fix([z[0] * -0.75 - y[0], z[1] * -0.75 - y[1], z[2] * -0.75 - y[2]]),
            fix([y[0] - z[0] * 0.75, y[1] - z[1] * 0.75, y[2] - z[2] * 0.75]),
        ];
        let dot_zyx = |a: V, n: V| a[2] * n[2] + a[1] * n[1] + a[0] * n[0];
        let reach = |base: f32, n: V, ext: V| -> f32 {
            let mut v = base;
            for k in 0..3 {
                v = if n[k] < 0.0 { v - n[k] * ext[k] } else { n[k] * ext[k] + v };
            }
            v
        };
        let first = [lo[0] + HALF, lo[1] + HALF, lo[2] + HALF];
        let mut limit = [0.0f32; 5];
        let mut acc = [0.0f32; 5];
        for (i, n) in planes.iter().enumerate() {
            let mut base = dot_zyx(o, *n);
            if i == 0 {
                base += d;
            }
            limit[i] = reach(base, *n, [HALF; 3]);
            acc[i] = dot_zyx(first, *n);
        }
        let mut active: Vec<Exclude> = Vec::new();
        for e in &w.excludes {
            let dv = [e.center[0] - o[0], e.center[1] - o[1], e.center[2] - o[2]];
            let dist = (dv[2] * dv[2] + dv[1] * dv[1] + dv[0] * dv[0]).sqrt();
            if !(dist <= e.radius + d) {
                continue;
            }
            let inside = planes.iter().enumerate().all(|(i, n)| {
                let mut base = dot_zyx(o, *n);
                if i == 0 {
                    base += d;
                }
                dot_zyx(e.center, *n) <= reach(base, *n, e.half)
            });
            if inside {
                active.push(*e);
            }
        }
        let ext_y = (hi[1] - lo[1]) + CUBE;
        let ext_z = (hi[2] - lo[2]) + CUBE;
        let step_x: Vec<f32> = planes.iter().map(|n| n[0] * CUBE - ext_y * n[1]).collect();
        let step_y: Vec<f32> = planes.iter().map(|n| n[1] * CUBE - ext_z * n[2]).collect();
        let step_z: Vec<f32> = planes.iter().map(|n| n[2] * CUBE).collect();
        let mut cells = Vec::new();
        let mut cx = lo[0];
        while cx <= hi[0] {
            let mut cy = lo[1];
            while cy <= hi[1] {
                let mut cz = lo[2];
                while cz <= hi[2] {
                    if (0..5).all(|i| acc[i] <= limit[i]) {
                        let excluded = active.iter().any(|e| e.min[0] <= cx && cx < e.max[0] && e.min[1] <= cy && cy < e.max[1] && e.min[2] <= cz && cz < e.max[2]);
                        if !excluded {
                            cells.push([cx, cy, cz]);
                        }
                    }
                    for i in 0..5 {
                        acc[i] += step_z[i];
                    }
                    cz += CUBE;
                }
                for i in 0..5 {
                    acc[i] += step_y[i];
                }
                cy += CUBE;
            }
            for i in 0..5 {
                acc[i] += step_x[i];
            }
            cx += CUBE;
        }
        let flakes = w.flakes.iter().take(w.active.max(0) as usize).map(|f| (f.pos, f.color)).collect();
        Some(WeatherDraw { texture: w.material.clone().unwrap_or_default(), mask: w.mask, size: w.size, size_max: w.size_max.min(MAX_POINT_SIZE), cells, flakes })
    }

    /// LevelInfo natives for the weather (ALevelInfo::exec…RndCubeSpr).
    pub(crate) fn weather_native(&mut self, name: &str, args: &[Value], given: impl Fn(usize) -> bool) -> Option<Value> {
        let f = |i: usize| args.get(i).map_or(0.0, |v| v.as_float());
        Some(match name {
            "InitRndCubeSpr" => {
                let texture = match args.first() {
                    Some(Value::Asset(p)) => Some(p.clone()),
                    _ => None,
                };
                self.rnd_cube_init(texture, args.get(1).map_or(0, |v| v.as_int()), f(2), f(3));
                Value::Void
            }
            "SetRndCubeSprState" => {
                self.rnd_cube_state(args.first().is_some_and(|v| v.as_bool()));
                Value::Void
            }
            "SetRndCubeSprSpeed" => {
                self.rnd_cube_speed(args.first().map_or([0.0; 3], |v| v.as_vector()), f(1), f(2));
                Value::Void
            }
            // Engine.dll's wrapper calls the method (vtable +0x78) without storing its return value in
            // Result: the script always reads false (Map06_HualparBase.Tick never disables itself).
            "ChangeRndCubeSprProp" => {
                self.rnd_cube_change(f(0), f(1), f(2));
                Value::Bool(false)
            }
            "AddRndCubeSprExclude" => {
                self.rnd_cube_exclude(args.first().map_or([0.0; 3], |v| v.as_vector()), args.get(1).map_or([0.0; 3], |v| v.as_vector()));
                Value::Void
            }
            "SetRndCubeSprSize" => {
                let max = if given(1) { f(1) } else { 64.0 };
                let mask = given(2) && args.get(2).is_some_and(|v| v.as_bool());
                self.rnd_cube_size(f(0), max, mask);
                Value::Void
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod static_text_tests {
    use super::RndCubeSpr;

    /// The snow statics written to the per-visit weather file are read back bit for bit; a damaged text changes nothing.
    #[test]
    fn static_text_go_return() {
        let mut a = RndCubeSpr { gust_dir: Some([0.25, -1.5, 3.0e-7]), wind: None, gust_time: 1.234_567, gust_period: 4.0, force_time: 0.1, force_period: 2.5, force_target: -0.75, force: 0.333, change_rest: 0.5, ..Default::default() };
        let s = a.static_text();
        let mut b = RndCubeSpr::default();
        assert!(b.static_place_text(&s));
        assert_eq!(b.static_text(), s);
        assert!(!b.static_place_text("abc;def"));
        assert_eq!(b.static_text(), s);
        a.wind = Some([1.0, 2.0, 3.0]);
        assert_ne!(a.static_text(), s);
    }
}
