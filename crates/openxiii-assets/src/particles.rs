//! Unreal 2 particle systems as Engine.dll runs and draws them.
//!
//! AEmitter::Tick runs each UParticleEmitter of the actor: Initialize,
//! Reset, SpawnParticles, SpawnParticle and
//! UpdateParticles. USpriteEmitter::FillVertexBuffer builds four
//! vertices per live particle, drawn by USpriteEmitter::RenderParticles with
//! a UParticleMaterial (DrawStyle, texture) translated by FD3DRenderInterface::SetParticleMaterial
//!
//! Computations are in single precision in machine-code order (24-bit FPU). appFrand
//! and appRand are Core's (msvcr70's rand()), on the sequence the caller provides: that of
//! the game thread, shared by script and HXAudio.
//! UBeamEmitter: Initialize, UpdateParticles, UpdateActorHitList
//! (done by the caller), SpawnParticle and RenderParticles
//!
//! Map collision (UseCollision): ULevel::SingleLineCheck from the old position to the
//! new one, TRACE_World (0x86), extent RealExtentMultiplier × Size, through the `trace` function
//! provided by the VM. Not reimplemented: the PTEP_TraceOffset trace (no XIII beam uses it);
//! beams in PTCS_Relative; a beam branch towards the emitter itself.

// Negated comparisons and bounds written as in the machine code (FCOM: NaN included).
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::needless_range_loop, clippy::manual_clamp, clippy::too_many_arguments)]

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::coords::rotate_angle_axis;
use crate::lighting::sin_u;

/// Microsoft C library rand(); the state is the game thread's _holdrand (lent by the VM).
#[derive(Debug, Clone)]
pub struct CRand(pub u32);

/// Number of draws made by particles (draw count per VM caller).
pub static DRAWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Callers of the draws, only kept when TRACE_DRAWS is true (the VM's draw log).
pub static TRACE_DRAWS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
thread_local! {
    pub static DRAW_CALLERS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl CRand {
    /// rand(): 0..=32767.
    #[track_caller]
    pub fn rand(&mut self) -> i32 {
        DRAWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if TRACE_DRAWS.load(std::sync::atomic::Ordering::Relaxed) {
            let line = std::panic::Location::caller().line();
            DRAW_CALLERS.with(|v| v.borrow_mut().push(line));
        }
        self.0 = self.0.wrapping_mul(214_013).wrapping_add(2_531_011);
        ((self.0 >> 16) & 0x7FFF) as i32
    }
    /// appFrand: (float)rand() × (1 / 32767).
    #[track_caller]
    pub fn frand(&mut self) -> f32 {
        self.rand() as f32 * 3.051_851e-5
    }
}

/// Range: Min, Max; draw (Min - Max) × appFrand() + Max.
#[derive(Debug, Clone, Copy, Default)]
pub struct Range {
    pub min: f32,
    pub max: f32,
}

impl Range {
    #[track_caller]
    pub fn get(&self, r: &mut CRand) -> f32 {
        (self.min - self.max) * r.frand() + self.max
    }
}

/// RangeVector: X, Y, Z; draws Y, then X, then Z.
#[derive(Debug, Clone, Copy, Default)]
pub struct RangeVector {
    pub x: Range,
    pub y: Range,
    pub z: Range,
}

impl RangeVector {
    #[track_caller]
    pub fn get(&self, r: &mut CRand) -> [f32; 3] {
        let y = self.y.get(r);
        let x = self.x.get(r);
        let z = self.z.get(r);
        [x, y, z]
    }
    /// All six bounds zero.
    fn is_zero(&self) -> bool {
        [self.x.min, self.x.max, self.y.min, self.y.max, self.z.min, self.z.max].iter().all(|&v| v == 0.0)
    }
}

/// Properties of a UParticleEmitter (and its subclasses) read for the simulation.
#[derive(Debug, Clone, Default)]
pub struct EmitterParams {
    pub kind: EmitterKind,
    pub acceleration: [f32; 3],
    pub use_collision: bool,
    pub use_collision_planes: bool,
    pub use_max_collisions: bool,
    pub use_spawned_velocity_scale: bool,
    pub use_color_scale: bool,
    pub fade_out: bool,
    pub fade_in: bool,
    pub respawn_dead_particles: bool,
    pub auto_destroy: bool,
    pub auto_reset: bool,
    pub disabled: bool,
    pub disable_fogging: bool,
    pub spin_particles: bool,
    pub damp_rotation: bool,
    pub use_size_scale: bool,
    pub use_regular_size_scale: bool,
    pub uniform_size: bool,
    pub automatic_initial_spawning: bool,
    pub use_subdivision_scale: bool,
    pub use_random_subdivision: bool,
    pub no_sync_anim: bool,
    pub once_texture_anim: bool,
    pub symmetry_u: bool,
    pub symmetry_v: bool,
    pub random_symmetry_u: bool,
    pub random_symmetry_v: bool,
    pub extent_multiplier: [f32; 3],
    pub damping_factor_range: RangeVector,
    pub collision_planes: Vec<[f32; 4]>,
    pub max_collisions: Range,
    pub spawn_from_other_emitter: i32,
    pub spawn_amount: i32,
    pub spawned_velocity_scale_range: RangeVector,
    /// (relative time, colour B, G, R, A).
    pub color_scale: Vec<(f32, [u8; 4])>,
    pub color_scale_repeats: f32,
    pub fade_out_factor: [f32; 4],
    pub fade_out_start_time: f32,
    pub fade_in_factor: [f32; 4],
    pub fade_in_end_time: f32,
    pub coordinate_system: u8,
    pub max_particles: i32,
    pub effect_axis: u8,
    pub auto_reset_time_range: Range,
    pub start_location_offset: [f32; 3],
    pub start_location_range: RangeVector,
    pub add_location_from_other_emitter: i32,
    pub start_location_shape: u8,
    pub sphere_radius_range: Range,
    pub start_mass_range: Range,
    pub use_rotation_from: u8,
    pub rotation_offset: [i32; 3],
    pub spin_ccw_or_cw: [f32; 3],
    pub spins_per_second_range: RangeVector,
    pub start_spin_range: RangeVector,
    pub rotation_damping_factor_range: RangeVector,
    pub rotation_normal: [f32; 3],
    /// (relative time, relative size).
    pub size_scale: Vec<(f32, f32)>,
    pub size_scale_repeats: f32,
    pub start_size_range: RangeVector,
    pub center_u: f32,
    pub center_v: f32,
    pub particles_per_second: f32,
    pub initial_particles_per_second: f32,
    pub draw_style: u8,
    pub texture: Option<String>,
    pub texture_u_subdivisions: i32,
    pub texture_v_subdivisions: i32,
    pub subdivision_scale: Vec<f32>,
    pub subdivision_start: i32,
    pub subdivision_end: i32,
    pub seconds_before_inactive: f32,
    pub min_squared_velocity: f32,
    pub initial_time_range: Range,
    pub lifetime_range: Range,
    pub initial_delay_range: Range,
    pub start_velocity_range: RangeVector,
    pub max_abs_velocity: [f32; 3],
    pub velocity_loss_range: RangeVector,
    pub add_velocity_from_other_emitter: i32,
    pub get_velocity_direction_from: u8,
    pub warmup_ticks_per_second: f32,
    pub relative_warmup_time: f32,
}

#[derive(Debug, Clone, Default)]
pub enum EmitterKind {
    /// USpriteEmitter: UseDirectionAs, ProjectionNormal.
    Sprite {
        use_direction_as: u8,
        projection_normal: [f32; 3],
    },
    /// UMeshEmitter: StaticMesh, UseMeshBlendMode, RenderTwoSided, ScaleAxis.
    Mesh {
        static_mesh: Option<String>,
        use_mesh_blend_mode: bool,
        render_two_sided: bool,
        scale_axis: u8,
        /// UPrimitive::BoundingBox of the StaticMesh (min, max).
        mesh_box: Option<([f32; 3], [f32; 3])>,
    },
    Beam(Box<BeamParams>),
    #[default]
    Other,
}

/// ParticleBeamEndPoint: ActorTag (resolved by the caller), Offset, Weight.
#[derive(Debug, Clone, Default)]
pub struct BeamEndPoint {
    pub offset: RangeVector,
    pub weight: f32,
    /// Location of the HitActors actor (UpdateActorHitList: first actor of the level whose
    /// Tag is ActorTag), read at spawn time.
    pub actor_location: Option<[f32; 3]>,
}

/// Properties of a UBeamEmitter.
#[derive(Debug, Clone, Default)]
pub struct BeamParams {
    pub determine_end_point_by: u8,
    pub beam_distance_range: Range,
    pub end_points: Vec<BeamEndPoint>,
    pub texture_u_scale: f32,
    pub texture_v_scale: f32,
    pub rotating_sheets: i32,
    pub low_frequency_points: i32,
    pub high_frequency_points: i32,
    pub low_frequency_noise_range: RangeVector,
    pub high_frequency_noise_range: RangeVector,
    /// ParticleBeamScale: FrequencyScale, RelativeLength.
    pub lf_scale_factors: Vec<([f32; 3], f32)>,
    pub hf_scale_factors: Vec<([f32; 3], f32)>,
    pub lf_scale_repeats: f32,
    pub hf_scale_repeats: f32,
    pub use_high_frequency_scale: bool,
    pub use_low_frequency_scale: bool,
    pub noise_determines_end_point: bool,
    pub use_branching: bool,
    pub branch_probability: Range,
    /// Already clamped to [0, Num - 1] if positive (UBeamEmitter::UpdateParticles).
    pub branch_emitter: i32,
    pub branch_spawn_amount_range: Range,
    pub linkup_lifetime: bool,
}

/// FParticle (0x88 bytes).
#[derive(Debug, Clone, Copy, Default)]
pub struct Particle {
    pub location: [f32; 3],
    pub old_location: [f32; 3],
    pub velocity: [f32; 3],
    pub start_size: [f32; 3],
    pub spins_per_second: [f32; 3],
    pub start_spin: [f32; 3],
    pub size: [f32; 3],
    pub start_location: [f32; 3],
    pub rotation: [f32; 3],
    /// B, G, R, A.
    pub color: [u8; 4],
    pub time: f32,
    pub max_lifetime: f32,
    pub mass: f32,
    pub hit_count: i32,
    /// 1: alive, 2: stationary, 4: spawned this frame, 8 / 0x10: U / V mirroring.
    pub flags: u32,
    pub subdivision: i32,
}

/// State of a UParticleEmitter held by the engine (transient properties).
#[derive(Debug, Clone, Default)]
pub struct EmitterState {
    pub particles: Vec<Particle>,
    pub particle_index: i32,
    pub active_particles: i32,
    pub pps_fraction: f32,
    pub real_extent_multiplier: [f32; 3],
    /// UMeshEmitter::MeshExtent (+0x2C4, transient): kept from one initialisation to the next.
    pub mesh_extent: [f32; 3],
    pub other_index: i32,
    pub initial_delay: f32,
    pub global_offset: [f32; 3],
    pub time_till_reset: f32,
    pub inactive_time: f32,
    pub initialized: bool,
    pub inactive: bool,
    pub real_disable_fogging: bool,
    pub all_particles_dead: bool,
    pub warmed_up: bool,
    /// Disabled may be set by the engine (AutoDestroy).
    pub disabled: bool,
    /// UParticleEmitter +0x2A8 / +0x2AC (NoSynchroAnim): current frame (AnimCurrent) and Accumulator of the texture
    /// animation specific to this emitter; `None`: not started yet (USpriteEmitter::RenderParticles).
    pub anim_cursor: Option<String>,
    pub anim_accumulator: f32,
    /// Texture (animation frame) with which the last render drew this emitter's sprites.
    pub anim_frame: Option<String>,
    /// Particles drawn by the render that just finished the emitter's one-shot animation (OnceTextureAnim):
    /// drawing precedes their death, so this frame's display shows them once more.
    pub last_draw: Option<Vec<Particle>>,
    pub real_projection_normal: [f32; 3],
    /// UBeamEmitter: BeamValueSum, SheetsUsed, HFPoints (X, Y, Z, abscissa) and LFPoints.
    pub beam_value_sum: f32,
    /// Native write of BranchEmitter by the last UpdateParticles, consumed by the VM.
    pub beam_branch_emitter: Option<i32>,
    pub sheets_used: i32,
    pub hf_points: Vec<[f32; 4]>,
    pub lf_points: Vec<[f32; 3]>,
    /// BranchEmitter of the same actor, lent for one step (branch spawns).
    pub branch_target: Option<Box<(Box<EmitterParams>, Box<EmitterState>)>>,
}

/// The Emitter actor, as its emitters see it.
#[derive(Debug, Clone, Default)]
pub struct EmitterOwner {
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    pub velocity: [f32; 3],
    pub global_offset: [f32; 3],
    pub auto_destroy: bool,
    pub auto_reset: bool,
    pub disable_fogging: bool,
    pub global_offset_range: RangeVector,
    pub time_till_reset_range: Range,
    pub time_till_reset: f32,
    /// The actor's bIgnoreFog (UParticleMaterial+0x60).
    pub ignore_fog: bool,
    /// The actor's PrePivot (pivot of UMeshEmitter meshes).
    pub pre_pivot: [f32; 3],
}

/// A UMeshEmitter particle to draw: transform and colour (B, G, R, A).
#[derive(Debug, Clone, Copy)]
pub struct MeshInstance {
    pub local_to_world: [[f32; 4]; 4],
    pub color: [u8; 4],
}

/// UMeshEmitter::RenderParticles: for each live particle, size chosen
/// by ScaleAxis (bit 0, 1, 2: Size instead of StartSize in X, Y, Z), truncated rotation
/// (StartSpin + Time × SpinsPerSecond, if SpinParticles), with the actor's
/// PrePivot; the particle colour is the global colour (TFACTOR).
/// _ftol: truncation to a 64-bit integer (out of range or NaN: 0x8000000000000000), low 32 bits.
fn ftol32(x: f32) -> i32 {
    let v = if x.is_nan() || x.abs() >= 9.223_372e18 { i64::MIN } else { x as i64 };
    v as i32
}

pub fn mesh_instances(p: &EmitterParams, s: &EmitterState, owner: &EmitterOwner) -> Vec<MeshInstance> {
    let EmitterKind::Mesh { scale_axis, .. } = p.kind else { return Vec::new() };
    let mut out = Vec::new();
    for q in s.particles.iter().take(s.active_particles.max(0) as usize) {
        if q.flags & 1 == 0 {
            continue;
        }
        let scale = [
            if scale_axis & 1 != 0 { q.size[0] } else { q.start_size[0] },
            if scale_axis & 2 != 0 { q.size[1] } else { q.start_size[1] },
            if scale_axis & 4 != 0 { q.size[2] } else { q.start_size[2] },
        ];
        // UMeshEmitter::RenderParticles: FLD spins * time, FADD start, then
        // _ftol (64-bit truncation, low 32 bits); Pitch = axis 1, Yaw = axis 0.
        let rotation = if p.spin_particles {
            [
                ftol32(q.spins_per_second[1] * q.time + q.start_spin[1]),
                ftol32(q.spins_per_second[0] * q.time + q.start_spin[0]),
                ftol32(q.spins_per_second[2] * q.time + q.start_spin[2]),
            ]
        } else {
            [0; 3]
        };
        let m = crate::lighting::local_to_world(rotation, scale, 1.0, q.location, owner.pre_pivot);
        out.push(MeshInstance { local_to_world: m, color: q.color });
    }
    out
}

/// X, Y, Z axes of FRotationMatrix (GMath tables): TransformVectorBy of
/// GMath.UnitCoords * Rotation brings a local vector into world space.
pub fn rotation_axes(r: [i32; 3]) -> [[f32; 3]; 3] {
    let (p, y, ro) = (r[0], r[1], r[2]);
    let sr = sin_u(-ro);
    let cr = sin_u(0x4000 - ro);
    let sy = sin_u(y);
    let cy = sin_u(y + 0x4000);
    let sp = sin_u(-p);
    let cp = sin_u(0x4000 - p);
    [[cp * cy, cp * sy, -sp], [sp * sr * cy - sy * cr, cy * cr + sp * sr * sy, cp * sr], [sy * sr + sp * cr * cy, sp * cr * sy - cy * sr, cp * cr]]
}

/// v expressed in the rotation's frame, brought into world space.
pub fn rotate(v: [f32; 3], r: [i32; 3]) -> [f32; 3] {
    let a = rotation_axes(r);
    [0, 1, 2].map(|k| v[0] * a[0][k] + v[1] * a[1][k] + v[2] * a[2][k])
}

/// FVector::Rotation: truncated yaw and pitch, zero roll. appAtan2 and appSqrt
/// (Core) are fpatan and fsqrt on the 24-bit FPU: each result is rounded to single, as is
/// the product by 32768 / π before truncation.
fn vector_rotation(v: [f32; 3]) -> [i32; 3] {
    let k = 10430.218905280077;
    let yaw = ((v[1] as f64).datan2(v[0] as f64) as f32 as f64 * k) as f32 as i32;
    let horiz = ((v[0] * v[0] + v[1] * v[1]) as f64).sqrt() as f32 as f64;
    let pitch = ((v[2] as f64).datan2(horiz) as f32 as f64 * k) as f32 as i32;
    [pitch, yaw, 0]
}

/// FVector::SafeNormal.
/// FVector::SafeNormal: (X² + Y²) + Z² compared with the double 1e-8, then × appRSqrt.
fn safe_normal(v: [f32; 3]) -> [f32; 3] {
    let sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if (sq as f64) < 1e-8 {
        return [0.0; 3];
    }
    let s = 1.0 / sq.sqrt();
    [v[0] * s, v[1] * s, v[2] * s]
}

/// VRand: random direction, length between 0.1 and 1 before normalisation.
#[track_caller]
fn vrand(r: &mut CRand) -> [f32; 3] {
    loop {
        let a = 1.0 - r.frand() * 2.0;
        let b = 1.0 - r.frand() * 2.0;
        let c = 1.0 - r.frand() * 2.0;
        // (C² + B²) + A², appSqrt (24-bit fsqrt).
        let len = ((c * c + b * b) + a * a).sqrt();
        if len > 1.0 || len < 0.1 {
            continue;
        }
        let inv = 1.0 / len;
        return [c * inv, b * inv, a * inv];
    }
}

/// appFractional, the engine's only fractional-part function (called by UBeamEmitter::SpawnParticle
/// and UParticleEmitter::UpdateParticles): FSUB 0.5 (rounded to 24 bits), FISTP round to nearest
/// even, FILD, FSUBR. This is not x - floor(x): odd integers (1, 3, -1, 16777215…) give 1.0, even ones 0.0.
/// FISTP outside 32-bit integers returns 0x80000000.
pub fn app_fractional(x: f32) -> f32 {
    let y = (x as f64 - 0.5) as f32;
    let r = y.round_ties_even();
    let n = if r.is_finite() && (-2147483648.0..2147483648.0).contains(&r) { r as i32 } else { i32::MIN };
    (x as f64 - n as f64) as f32
}

/// FISTP round to nearest.
fn fistp(x: f32) -> i32 {
    x.round_ties_even() as i32
}

impl EmitterState {
    /// UParticleEmitter::Reset.
    pub fn reset(&mut self, p: &EmitterParams, r: &mut CRand) {
        self.active_particles = 0;
        self.particle_index = 0;
        self.other_index = 0;
        self.all_particles_dead = false;
        self.warmed_up = false;
        self.initial_delay = p.initial_delay_range.get(r);
        self.time_till_reset = p.auto_reset_time_range.get(r);
    }

    /// UParticleEmitter::CleanUp: Particles emptied, ActiveParticles and ParticleIndex
    /// set to 0, Initialized cleared; the rest (AllParticlesDead, Inactive…) is unchanged.
    /// USpriteEmitter::CleanUp only calls the base; UBeamEmitter::CleanUp
    /// also empties its point arrays (HF, LF) first.
    pub fn clean_up(&mut self) {
        self.particles.clear();
        self.active_particles = 0;
        self.particle_index = 0;
        self.initialized = false;
    }

    /// UParticleEmitter::Initialize (and USpriteEmitter::Initialize: RealProjectionNormal
    /// = ProjectionNormal.SafeNormal(), FVector::SafeNormal).
    pub fn initialize(&mut self, p: &EmitterParams, r: &mut CRand) {
        self.particles = vec![Particle::default(); p.max_particles.max(0) as usize];
        self.reset(p, r);
        self.initialized = true;
        if let EmitterKind::Sprite { projection_normal, .. } = p.kind {
            self.real_projection_normal = safe_normal(projection_normal);
        }
        // UMeshEmitter::Initialize: with a StaticMesh, MeshExtent = max(Min, Max) −
        // box centre; with or without one (outside the mesh test),
        // RealExtentMultiplier = ExtentMultiplier × MeshExtent (MeshExtent is transient: zero or
        // that of a previous initialisation).
        if let EmitterKind::Mesh { mesh_box, .. } = p.kind {
            if let Some((lo, hi)) = mesh_box {
                let m = [0, 1, 2].map(|k| if hi[k] <= lo[k] { lo[k] } else { hi[k] });
                let c = [(hi[0] + lo[0]) * 0.5, (hi[1] + lo[1]) * 0.5, (hi[2] + lo[2]) * 0.5];
                self.mesh_extent = [m[0] - c[0], m[1] - c[1], m[2] - c[2]];
            }
            let (x, e) = (p.extent_multiplier, self.mesh_extent);
            self.real_extent_multiplier = [x[0] * e[0], e[1] * x[1], e[2] * x[2]];
        }
        if let EmitterKind::Beam(b) = &p.kind {
            self.sheets_used = if b.rotating_sheets == 0 { 1 } else { b.rotating_sheets };
            let n = p.max_particles.max(0) as usize;
            self.hf_points = vec![[0.0; 4]; b.high_frequency_points.max(0) as usize * n];
            self.lf_points = vec![[0.0; 3]; b.low_frequency_points.max(0) as usize * n];
        }
    }

    /// UParticleEmitter::SpawnParticles: spawns for the frame, spread over dt. Returns the
    /// new fraction.
    fn spawn_particles(&mut self, p: &EmitterParams, others: &[OtherEmitter], owner: &EmitterOwner, fraction: f32, pps: f32, dt: f32, r: &mut CRand) -> f32 {
        if !(pps > 0.0) {
            return 0.0;
        }
        let total = pps * dt + fraction;
        let mut n = fistp(total - 0.5);
        let rest = total - n as f32;
        // MaxParticles zero: no spawn (n clamped to 0), the rest is kept.
        if p.max_particles == 0 {
            return rest;
        }
        let step = 1.0 / pps;
        let first = (step * fraction + dt) - step;
        if n < 0 {
            n = 0;
        } else if p.max_particles <= n {
            n = p.max_particles;
        }
        for i in 0..n {
            self.spawn_particle(p, others, owner, self.particle_index, first - i as f32 * step, 4, 0, [0.0; 3], r);
            self.active_particles = self.active_particles.max(self.particle_index + 1);
            self.particle_index = (self.particle_index + 1) % p.max_particles;
        }
        rest
    }

    /// UParticleEmitter::execSpawnParticle: `count` immediate spawns (script).
    pub fn spawn_now(&mut self, p: &EmitterParams, others: &[OtherEmitter], owner: &EmitterOwner, count: i32, r: &mut CRand) {
        // execSpawnParticle clamps the count to MaxParticles then, for each one, calls SpawnParticle
        // (which returns early if MaxParticles or Owner is null, not based on Initialized) and advances
        // ParticleIndex and ActiveParticles whatever it did.
        if p.max_particles == 0 {
            return;
        }
        let n = count.clamp(0, p.max_particles);
        for _ in 0..n {
            self.spawn_particle(p, others, owner, self.particle_index, 0.0, 0, 0, [0.0; 3], r);
            self.active_particles = self.active_particles.max(self.particle_index + 1);
            self.particle_index = (self.particle_index + 1) % p.max_particles;
        }
    }

    /// SpawnParticle (virtual): UParticleEmitter::SpawnParticle, then the rest of
    /// UBeamEmitter::SpawnParticle for a beam.
    #[allow(clippy::too_many_arguments)]
    fn spawn_particle(
        &mut self,
        p: &EmitterParams,
        others: &[OtherEmitter],
        owner: &EmitterOwner,
        index: i32,
        dt: f32,
        new_flags: u32,
        location_flags: u32,
        extra: [f32; 3],
        r: &mut CRand,
    ) {
        self.spawn_particle_base(p, others, owner, index, dt, new_flags, location_flags, extra, r);
        if let EmitterKind::Beam(b) = &p.kind {
            if p.max_particles != 0 && index >= 0 && (index as usize) < self.particles.len() {
                self.spawn_beam(b, others, owner, index as usize, dt, r);
            }
        }
    }

    /// UBeamEmitter::SpawnParticle after UParticleEmitter::SpawnParticle: beam
    /// end point (DetermineEndPointBy), the particle's low-frequency points then high-frequency
    /// points interpolated between them, noise included; branches towards BranchEmitter.
    fn spawn_beam(&mut self, b: &BeamParams, others: &[OtherEmitter], owner: &EmitterOwner, index: usize, dt: f32, r: &mut CRand) {
        let mut end = [0.0f32; 3];
        let mut offset = [0.0f32; 3];
        let mut chosen: i32 = -1;
        let mut set_end = false;
        let mode = b.determine_end_point_by;
        if 1 < mode && mode < 5 {
            if self.beam_value_sum == 0.0 {
                if !b.end_points.is_empty() {
                    chosen = 0;
                }
            } else {
                let target = r.frand() * self.beam_value_sum;
                let mut acc = 0.0f32;
                for (i, ep) in b.end_points.iter().enumerate() {
                    if !(acc >= target) && !(acc + ep.weight < target) {
                        chosen = i as i32;
                        break;
                    }
                    acc += ep.weight;
                }
            }
        }
        let q = self.particles[index];
        match mode {
            0 => {
                let m = q.max_lifetime;
                offset = [m * q.velocity[0], m * q.velocity[1], m * q.velocity[2]];
            }
            1 => {
                let v = normalize(q.velocity);
                self.particles[index].velocity = v;
                let d = b.beam_distance_range.get(r);
                offset = [d * v[0], d * v[1], d * v[2]];
            }
            2 | 4 => {
                if chosen != -1 {
                    let o = b.end_points[chosen as usize].offset.get(r);
                    end = [o[0] + q.location[0], o[1] + q.location[1], o[2] + q.location[2]];
                    offset = o;
                    // PTEP_TraceOffset: ULevel::SingleLineCheck from the start point to the end,
                    // not reimplemented (the end stays the Offset's, as without an obstacle).
                }
                set_end = true;
            }
            3 => {
                if chosen != -1 {
                    if let Some(a) = b.end_points[chosen as usize].actor_location {
                        offset = [a[0] - q.location[0], a[1] - q.location[1], a[2] - q.location[2]];
                        end = a;
                    }
                }
                set_end = true;
            }
            _ => {}
        }
        if b.noise_determines_end_point {
            set_end = false;
        }
        let lf = b.low_frequency_points;
        let hf = b.high_frequency_points;
        if lf <= 0 || hf <= 0 {
            return;
        }
        let step = 1.0 / (hf - 1) as f32;
        let inv_lf1 = 1.0 / (lf - 1) as f32;
        let inv_lf = 1.0 / lf as f32;
        let inv_hf = 1.0 / hf as f32;
        let lf_base = lf as usize * index;
        let loc = self.particles[index].location;
        self.lf_points[lf_base] = loc;
        let mut noise = [0.0f32; 3];
        let mut last = lf_base;
        for i in 1..lf {
            if !b.use_low_frequency_scale {
                noise = b.low_frequency_noise_range.get(r);
            } else {
                let s = app_fractional((b.lf_scale_repeats + 1.0) * i as f32 * inv_lf);
                // The machine code draws from HighFrequencyNoiseRange here.
                if let Some(n) = beam_scale(&b.lf_scale_factors, s, &b.high_frequency_noise_range, r) {
                    noise = n;
                }
            }
            let fi = i as f32;
            last = lf_base + i as usize;
            self.lf_points[last] =
                [offset[0] * fi * inv_lf1 + loc[0] + noise[0], offset[1] * fi * inv_lf1 + loc[1] + noise[1], offset[2] * fi * inv_lf1 + loc[2] + noise[2]];
        }
        if set_end {
            self.lf_points[last] = end;
        }
        let hf_base = hf as usize * index;
        let first = self.lf_points[lf_base];
        self.hf_points[hf_base] = [first[0], first[1], first[2], 0.0];
        let mut k = lf_base;
        let mut t = step;
        let mut noise = [0.0f32; 3];
        let mut last = hf_base;
        let beam = self.particles[index];
        for i in 1..hf {
            if !b.use_high_frequency_scale {
                noise = b.high_frequency_noise_range.get(r);
            } else {
                let s = app_fractional((b.hf_scale_repeats + 1.0) * i as f32 * inv_hf);
                if let Some(n) = beam_scale(&b.hf_scale_factors, s, &b.high_frequency_noise_range, r) {
                    noise = n;
                }
            }
            let f = (lf - 1) as f32 * t;
            let g = 1.0 - f;
            let a = self.lf_points.get(k).copied().unwrap_or_default();
            let c = self.lf_points.get(k + 1).copied().unwrap_or_default();
            let point = [g * a[0] + f * c[0] + noise[0], g * a[1] + f * c[1] + noise[1], g * a[2] + f * c[2] + noise[2]];
            last = hf_base + i as usize;
            self.hf_points[last] = [point[0], point[1], point[2], i as f32 * step];
            if b.use_branching {
                let prob = b.branch_probability.get(r);
                let f2 = r.frand();
                if !(prob < f2) && b.branch_emitter >= 0 {
                    if let Some(target) = self.branch_target.as_mut() {
                        let (tp, ts) = &mut **target;
                        if ts.initialized {
                            let count = b.branch_spawn_amount_range.get(r) as i32;
                            for _ in 0..count {
                                ts.spawn_particle(tp, others, owner, ts.particle_index, dt, 0, 3, point, r);
                                if b.linkup_lifetime {
                                    if let Some(x) = ts.particles.get_mut(ts.particle_index as usize) {
                                        x.time = beam.time;
                                        x.max_lifetime = beam.max_lifetime;
                                    }
                                }
                                ts.active_particles = ts.active_particles.max(ts.particle_index + 1);
                                if tp.max_particles != 0 {
                                    ts.particle_index = (ts.particle_index + 1) % tp.max_particles;
                                }
                            }
                        }
                    }
                }
            }
            t += step;
            if inv_lf1 <= t {
                k += 1;
                t -= inv_lf1;
            }
        }
        if set_end {
            let w = self.hf_points[last][3];
            self.hf_points[last] = [end[0], end[1], end[2], w];
        }
    }

    /// UParticleEmitter::SpawnParticle.
    #[allow(clippy::too_many_arguments)]
    fn spawn_particle_base(
        &mut self,
        p: &EmitterParams,
        others: &[OtherEmitter],
        owner: &EmitterOwner,
        index: i32,
        dt: f32,
        new_flags: u32,
        location_flags: u32,
        extra: [f32; 3],
        r: &mut CRand,
    ) {
        if p.max_particles == 0 || index < 0 || index as usize >= self.particles.len() {
            return;
        }
        let mut q = Particle { location: p.start_location_offset, ..self.particles[index as usize] };
        match p.start_location_shape {
            0 => {
                let d = p.start_location_range.get(r);
                q.location = [q.location[0] + d[0], q.location[1] + d[1], q.location[2] + d[2]];
            }
            1 => {
                let radius = p.sphere_radius_range.get(r);
                let d = vrand(r);
                q.location = [radius * d[0] + q.location[0], radius * d[1] + q.location[1], radius * d[2] + q.location[2]];
            }
            _ => {}
        }
        self.other_index += 1;
        if p.add_location_from_other_emitter >= 0 {
            // UpdateParticles clamps the index to Owner.Emitters.Num() − 1.
            if let Some(o) = others.get((p.add_location_from_other_emitter as usize).min(others.len().saturating_sub(1))) {
                if o.active_particles > 0 {
                    let src = o.locations[(self.other_index % o.active_particles) as usize];
                    q.location = [
                        (src[0] - owner.location[0]) + q.location[0],
                        (src[1] - owner.location[1]) + q.location[1],
                        (src[2] - owner.location[2]) + q.location[2],
                    ];
                }
            }
        }
        // Frame composed as in the original (for the position, for the velocity):
        // GMath.UnitCoords * RotationOffset (FCoords * FRotator), then
        // * Owner.Rotation for PTRS_Actor, and a single TransformVectorBy. Two
        // successive vector rotations do not give the same rounding.
        // `position`: PTRS_Normal removes 0x4000 from the pitch for EffectAxis PTEA_PositiveZ for the
        // position only; the velocity takes RotationNormal's rotation as
        // is.
        let frame = |p: &EmitterParams, position: bool| -> Option<Transform> {
            use crate::coords::{mul_rotator, transform_vector, UNIT};
            let c = match p.use_rotation_from {
                1 => mul_rotator(&mul_rotator(&UNIT, p.rotation_offset), owner.rotation),
                2 => mul_rotator(&UNIT, p.rotation_offset),
                3 => {
                    let mut rot = vector_rotation(p.rotation_normal);
                    if position && p.effect_axis == 1 {
                        rot[0] -= 0x4000;
                    }
                    mul_rotator(&UNIT, rot)
                }
                _ => return None,
            };
            Some(Box::new(move |v| transform_vector(v, &c)))
        };
        if let Some(f) = frame(p, true) {
            q.location = f(q.location);
        }
        if p.coordinate_system == 0 && location_flags & 2 == 0 {
            let o = owner;
            q.location = [
                dt * o.velocity[0] + o.location[0] + q.location[0],
                dt * o.velocity[1] + o.location[1] + q.location[1],
                (dt * o.velocity[2] + o.location[2]) + q.location[2],
            ];
        }
        if location_flags & 1 == 0 {
            q.location = [q.location[0] + self.global_offset[0], self.global_offset[1] + q.location[1], self.global_offset[2] + q.location[2]];
        }
        q.location = [q.location[0] + extra[0], extra[1] + q.location[1], extra[2] + q.location[2]];
        q.old_location = q.location;
        q.start_location = q.location;
        q.rotation = [0.0; 3];
        q.velocity = p.start_velocity_range.get(r);
        q.max_lifetime = p.lifetime_range.get(r);
        let t = r.frand();
        q.hit_count = 0;
        q.flags = new_flags | 1;
        q.time = (p.initial_time_range.min - p.initial_time_range.max) * t + p.initial_time_range.max + dt;
        q.mass = p.start_mass_range.get(r);
        q.start_size = p.start_size_range.get(r);
        if p.uniform_size {
            q.start_size[1] = q.start_size[0];
            q.start_size[2] = q.start_size[0];
        }
        q.size = q.start_size;
        q.velocity = [dt * p.acceleration[0] + q.velocity[0], dt * p.acceleration[1] + q.velocity[1], dt * p.acceleration[2] + q.velocity[2]];
        if let Some(f) = frame(p, false) {
            q.velocity = f(q.velocity);
        }
        let mul = |n: [f32; 3], v: [f32; 3]| [n[0] * v[0], n[1] * v[1], n[2] * v[2]];
        if p.coordinate_system == 1 {
            match p.get_velocity_direction_from {
                1 => q.velocity = mul(safe_normal(q.location), q.velocity),
                2 => q.velocity = mul(safe_normal([-q.location[0], -q.location[1], -q.location[2]]), q.velocity),
                _ => {}
            }
        } else {
            let o = owner.location;
            match p.get_velocity_direction_from {
                1 => q.velocity = mul(safe_normal([q.location[0] - o[0], q.location[1] - o[1], q.location[2] - o[2]]), q.velocity),
                2 => q.velocity = mul(safe_normal([o[0] - q.location[0], o[1] - q.location[1], o[2] - q.location[2]]), q.velocity),
                _ => {}
            }
        }
        if p.add_velocity_from_other_emitter >= 0 {
            if let Some(o) = others.get((p.add_velocity_from_other_emitter as usize).min(others.len().saturating_sub(1))) {
                if o.active_particles > 0 {
                    let v = o.velocities[(self.other_index % o.active_particles) as usize];
                    q.velocity = [q.velocity[0] + v[0], v[1] + q.velocity[1], v[2] + q.velocity[2]];
                }
            }
        }
        q.location = [dt * q.velocity[0] + q.location[0], dt * q.velocity[1] + q.location[1], dt * q.velocity[2] + q.location[2]];
        if q.mass != 0.0 {
            q.mass = 1.0 / q.mass;
        }
        q.start_spin = p.start_spin_range.get(r);
        q.spins_per_second = p.spins_per_second_range.get(r);
        for k in 0..3 {
            if r.frand() < p.spin_ccw_or_cw[k] {
                q.spins_per_second[k] *= -1.0;
            }
        }
        for k in 0..3 {
            q.start_spin[k] *= 65535.0;
        }
        for k in 0..3 {
            q.spins_per_second[k] *= 65535.0;
        }
        q.subdivision = if !p.use_random_subdivision {
            -1
        } else if p.subdivision_end != 0 {
            let f = r.frand();
            ((p.subdivision_end - p.subdivision_start) as f32 * f + p.subdivision_start as f32) as i32
        } else {
            let f = r.frand();
            ((p.texture_v_subdivisions * p.texture_u_subdivisions) as f32 * f) as i32
        };
        if p.symmetry_u && (!p.random_symmetry_u || r.rand() % 2 == 1) {
            q.flags |= 8;
        }
        if p.symmetry_v && (!p.random_symmetry_v || r.rand() % 2 == 1) {
            q.flags |= 0x10;
        }
        self.particles[index as usize] = q;
        if q.max_lifetime < q.time {
            self.spawn_particle(p, others, owner, index, dt - q.max_lifetime, 0, 0, [0.0; 3], r);
        }
        self.all_particles_dead = false;
    }

    /// UParticleEmitter::UpdateParticles (preceded, for a beam, by the sum of Weights of
    /// UBeamEmitter::UpdateParticles; UMeshEmitter::UpdateParticles only calls
    /// the base). Returns the number of live particles.
    pub fn update_particles(&mut self, p: &EmitterParams, others: &[OtherEmitter], owner: &EmitterOwner, dt: f32, r: &mut CRand, trace: &mut WorldTrace) -> i32 {
        if let EmitterKind::Beam(b) = &p.kind {
            self.beam_branch_emitter = Some(b.branch_emitter);
            self.beam_value_sum = 0.0;
            for ep in &b.end_points {
                self.beam_value_sum += ep.weight;
            }
        }
        let mut dead = 0;
        let mut pps = if self.active_particles < p.max_particles {
            if p.automatic_initial_spawning {
                p.max_particles as f32 / ((p.lifetime_range.max + p.lifetime_range.min) * 0.5)
            } else {
                p.initial_particles_per_second
            }
        } else {
            p.particles_per_second
        };
        if !pps.is_finite() {
            pps = 0.0;
        }
        if pps > 0.0 {
            self.pps_fraction = self.spawn_particles(p, others, owner, self.pps_fraction, pps, dt, r);
        }
        let local = p.coordinate_system == 1;
        let velocity_loss = !p.velocity_loss_range.is_zero();
        let clamp_velocity = !(p.max_abs_velocity[0] == 0.0 && p.max_abs_velocity[1] == 0.0 && p.max_abs_velocity[2] == 0.0);
        let mut i = 0;
        while i < self.active_particles {
            let idx = i as usize;
            let flags = self.particles[idx].flags;
            if flags & 1 == 0 {
                dead += 1;
                i += 1;
                continue;
            }
            let mut moving = flags & 2 == 0 || local;
            {
                let q = &mut self.particles[idx];
                if flags & 4 == 0 {
                    q.time += dt;
                } else {
                    moving = false;
                    q.flags = flags & !4;
                }
            }
            let alive = self.particles[idx].time <= self.particles[idx].max_lifetime;
            if !alive {
                if p.respawn_dead_particles {
                    let q = self.particles[idx];
                    let mut t = (q.time - q.max_lifetime) + p.initial_time_range.get(r);
                    while q.max_lifetime < t {
                        t -= q.max_lifetime;
                    }
                    self.spawn_particle(p, others, owner, i, t, 0, 0, [0.0; 3], r);
                } else {
                    dead += 1;
                    self.particles[idx].flags &= !1;
                    i += 1;
                    continue;
                }
            } else if moving {
                let q = &mut self.particles[idx];
                q.velocity[0] += dt * p.acceleration[0];
                q.velocity[1] += dt * p.acceleration[1];
                q.old_location = q.location;
                q.velocity[2] += dt * p.acceleration[2];
                for k in 0..3 {
                    q.location[k] += dt * q.velocity[k];
                }
            }
            // Collision: the map if UseCollision (SingleLineCheck, contact and
            // normal); otherwise, or without contact, the CollisionPlanes if UseCollisionPlanes.
            if moving && !local && (p.use_collision || p.use_collision_planes) {
                let q = self.particles[idx];
                let mut hit: Option<([f32; 3], [f32; 3])> = None;
                if p.use_collision {
                    let e = self.real_extent_multiplier;
                    hit = trace(q.location, q.old_location, [e[0] * q.size[0], e[1] * q.size[1], e[2] * q.size[2]]);
                }
                let planes: &[[f32; 4]] = if hit.is_none() && p.use_collision_planes { &p.collision_planes } else { &[] };
                for plane in planes {
                    // (Z × N.Z + Y × N.Y) + X × N.X − W, then (sign).
                    let side = |v: [f32; 3]| {
                        let d = ((v[2] * plane[2] + v[1] * plane[1]) + v[0] * plane[0]) - plane[3];
                        if d > 0.0 {
                            1
                        } else if d < 0.0 {
                            -1
                        } else {
                            0
                        }
                    };
                    if side(q.old_location) != side(q.location) {
                        let a = q.location;
                        let b = q.old_location;
                        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                        let t = (plane[3] - ((a[0] * plane[0] + plane[2] * a[2]) + plane[1] * a[1])) / ((d[2] * plane[2] + d[1] * plane[1]) + d[0] * plane[0]);
                        hit = Some(([d[0] * t + a[0], d[1] * t + a[1], d[2] * t + a[2]], [plane[0], plane[1], plane[2]]));
                        break;
                    }
                }
                if let Some((point, normal)) = hit {
                    self.collide(p, others, owner, i, point, normal, dt, r);
                    if self.particles[idx].flags & 1 == 0 {
                        dead += 1;
                        i += 1;
                        continue;
                    }
                }
            }
            let q = &mut self.particles[idx];
            let time = q.time;
            let rel = if q.max_lifetime == 0.0 {
                0.0
            } else {
                let x = time / q.max_lifetime;
                if x < 0.0 {
                    0.0
                } else if 1.0 <= x {
                    1.0
                } else {
                    x
                }
            };
            if p.use_size_scale {
                let s = if p.use_regular_size_scale {
                    1.0 / (q.time + 1.0)
                } else if q.max_lifetime != 0.0 && !p.size_scale.is_empty() {
                    let t = app_fractional((p.size_scale_repeats + 1.0) * rel);
                    let sc = &p.size_scale;
                    if sc[0].0 < t {
                        let mut j = 0;
                        let mut found = None;
                        while j + 1 < sc.len() {
                            if sc[j + 1].0 >= t {
                                found = Some(j);
                                break;
                            }
                            j += 1;
                        }
                        match found {
                            Some(j) => {
                                let (a, b) = (sc[j], sc[j + 1]);
                                if b.0 - a.0 <= 0.0 {
                                    (b.1 - a.1) * 1.0 + a.1
                                } else {
                                    (b.1 - a.1) * ((t - a.0) / (b.0 - a.0)) + a.1
                                }
                            }
                            None => sc[sc.len() - 1].1,
                        }
                    } else {
                        sc[0].1
                    }
                } else {
                    1.0
                };
                q.size = [s * q.start_size[0], s * q.start_size[1], s * q.start_size[2]];
            }
            let mut color = [0xFFu8; 4];
            if p.use_color_scale && q.max_lifetime != 0.0 && !p.color_scale.is_empty() {
                let t = app_fractional((p.color_scale_repeats + 1.0) * rel);
                let cs = &p.color_scale;
                if cs[0].0 < t {
                    let mut out = None;
                    for j in 1..cs.len() {
                        if t <= cs[j].0 {
                            let (a, b) = (cs[j - 1], cs[j]);
                            let span = cs[j].0 - cs[j - 1].0;
                            let f = if span <= 0.0 { 1.0 } else { (t - cs[j - 1].0) / span };
                            let lerp = |x: u8, y: u8| ((y as i32 - x as i32) as f32 * f + x as i32 as f32) as i32 as u8;
                            out = Some([lerp(a.1[0], b.1[0]), lerp(a.1[1], b.1[1]), lerp(a.1[2], b.1[2]), 0xFF]);
                            break;
                        }
                    }
                    color = out.unwrap_or({
                        let c = cs[cs.len() - 1].1;
                        [c[0], c[1], c[2], 0xFF]
                    });
                } else {
                    let c = cs[0].1;
                    color = [c[0], c[1], c[2], 0xFF];
                }
            }
            // Fade: factor × (time elapsed in the phase) × 255, depending on the DrawStyle.
            let fade = if p.fade_out && time > p.fade_out_start_time && q.max_lifetime != p.fade_out_start_time {
                Some((p.fade_out_factor, (time - p.fade_out_start_time) / (q.max_lifetime - p.fade_out_start_time) * 255.0))
            } else if p.fade_in && time < p.fade_in_end_time && p.fade_in_end_time != 0.0 {
                Some((p.fade_in_factor, (p.fade_in_end_time - time) / p.fade_in_end_time * 255.0))
            } else {
                None
            };
            if let Some((factor, f)) = fade {
                match p.draw_style {
                    2 => {
                        color[0] = 0x7F;
                        color[1] = 0x7F;
                        color[2] = 0x7F;
                        color[3] = 0xFFu8.wrapping_sub((factor[3] * f) as i32 as u8);
                    }
                    1 => {
                        let a = (color[3] as f32 - factor[3] * f).clamp(0.0, 255.0);
                        color[3] = a as i32 as u8;
                    }
                    _ => {
                        let c = |v: u8, k: f32| ((v as f32 - k * f) as i32).max(0) as u8;
                        let (r_, g, b, a) = (c(color[2], factor[0]), c(color[1], factor[1]), c(color[0], factor[2]), c(color[3], factor[3]));
                        color = [b, g, r_, a];
                    }
                }
            }
            q.color = color;
            if clamp_velocity {
                for k in 0..3 {
                    let m = p.max_abs_velocity[k];
                    if m != 0.0 {
                        let v = q.velocity[k];
                        q.velocity[k] = if -m <= v {
                            if v < m {
                                v
                            } else {
                                m
                            }
                        } else {
                            -m
                        };
                    }
                }
            }
            if velocity_loss {
                let l = p.velocity_loss_range.get(r);
                let q = &mut self.particles[idx];
                let z = l[2] * q.velocity[2];
                let x = q.velocity[0] * l[0] * dt;
                q.velocity[0] -= x;
                q.velocity[1] -= l[1] * q.velocity[1] * dt;
                q.velocity[2] -= z * dt;
            }
            i += 1;
        }
        self.all_particles_dead = (dead == p.max_particles || (self.active_particles == dead && pps == 0.0)) && !p.respawn_dead_particles;
        self.active_particles - dead
    }

    /// Particle bounce (collision branch of UpdateParticles): half a step of acceleration
    /// removed, velocity reflected × DampingFactorRange, position at the contact
    /// point + 0.01 × normal; MaxCollisions. SpawnFromOtherEmitter is not reimplemented: no XIII
    /// package or map gives it a value (the name only appears in Engine.u).
    #[allow(clippy::too_many_arguments)]
    fn collide(&mut self, p: &EmitterParams, others: &[OtherEmitter], owner: &EmitterOwner, i: i32, point: [f32; 3], normal: [f32; 3], dt: f32, r: &mut CRand) {
        let idx = i as usize;
        if p.use_max_collisions {
            let max = fistp(p.max_collisions.get(r) - 0.5);
            if self.particles[idx].hit_count + 1 >= max {
                if !p.respawn_dead_particles {
                    self.particles[idx].flags &= !1;
                    return;
                }
                // Respawn, then the same sequence as after a bounce: the
                // MinSquaredVelocity test on the new particle.
                self.spawn_particle(p, others, owner, i, dt * 0.5, 0, 0, [0.0; 3], r);
                self.min_squared_velocity(p, idx);
                return;
            }
        }
        let q = &mut self.particles[idx];
        let half = [dt * p.acceleration[0] * 0.5, dt * p.acceleration[1] * 0.5, dt * p.acceleration[2] * 0.5];
        for k in 0..3 {
            q.velocity[k] -= half[k];
        }
        let v = q.velocity;
        let d = ((normal[1] * v[1] + normal[0] * v[0]) + v[2] * normal[2]) * 2.0;
        q.velocity = [v[0] - d * normal[0], v[1] - d * normal[1], v[2] - d * normal[2]];
        let damp = p.damping_factor_range.get(r);
        let q = &mut self.particles[idx];
        for k in 0..3 {
            q.velocity[k] *= damp[k];
        }
        if p.damp_rotation {
            let rd = p.rotation_damping_factor_range.get(r);
            let q = &mut self.particles[idx];
            for k in 0..3 {
                q.spins_per_second[k] *= rd[k];
            }
        }
        let q = &mut self.particles[idx];
        let at = [normal[0] * 0.01 + point[0], normal[1] * 0.01 + point[1], point[2] + normal[2] * 0.01];
        q.location = at;
        q.old_location = at;
        q.hit_count += 1;
        self.min_squared_velocity(p, idx);
    }

    /// End of the collision branch: below MinSquaredVelocity ((X² + Y²) + Z²), the
    /// particle stops moving (bit 2) and stops spinning.
    fn min_squared_velocity(&mut self, p: &EmitterParams, idx: usize) {
        let q = &mut self.particles[idx];
        if p.min_squared_velocity != 0.0 {
            let v = q.velocity;
            if (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2] < p.min_squared_velocity {
                q.flags |= 2;
                q.spins_per_second = [0.0; 3];
            }
        }
    }
}

/// LFScaleFactors / HFScaleFactors at abscissa `s`: first factor whose
/// RelativeLength ≥ s, interpolated from the previous one ((1, 1, 1) and 0 before the first), times
/// a draw of `noise`. No factor: the previous noise remains.
fn beam_scale(factors: &[([f32; 3], f32)], s: f32, noise: &RangeVector, r: &mut CRand) -> Option<[f32; 3]> {
    for (j, &(scale, length)) in factors.iter().enumerate() {
        if s <= length {
            let (prev, prev_length) = if j == 0 { ([1.0f32; 3], 0.0f32) } else { factors[j - 1] };
            let a = if length == 0.0 { 1.0 } else { (s - prev_length) / (length - prev_length) };
            let n = noise.get(r);
            return Some([
                ((scale[0] - prev[0]) * a + prev[0]) * n[0],
                ((scale[1] - prev[1]) * a + prev[1]) * n[1],
                ((scale[2] - prev[2]) * a + prev[2]) * n[2],
            ]);
        }
    }
    None
}

/// UBeamEmitter::RenderParticles: for each live particle and each sheet,
/// two vertices per high-frequency point, on either side of the point along
/// (point - previous) × (previous - camera) normalised, rotated by 0x8000 / SheetsUsed × sheet
/// around the segment, times Size.X; U = BeamTextureUScale × abscissa, V = 0 or
/// BeamTextureVScale. Indices: b, b+1, b+2, b+1, b+3, b+2 per segment. In the relative frame
/// (CoordinateSystem 1), vertices are local: the engine draws with LocalToWorld (see
/// relative_to_world).
pub fn fill_beam_vertices(p: &EmitterParams, s: &EmitterState, owner: &EmitterOwner, view: &ViewFrame) -> (Vec<SpriteVertex>, Vec<u32>) {
    let EmitterKind::Beam(b) = &p.kind else { return (Vec::new(), Vec::new()) };
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut cam = view.location;
    if p.coordinate_system == 1 {
        // Camera through TransformPointBy(GMath.UnitCoords / Owner.Rotation + Owner.Location)
        use crate::coords;
        let mut c = coords::div_rotator(&coords::UNIT, owner.rotation);
        c.origin = [owner.location[0] + c.origin[0], owner.location[1] + c.origin[1], owner.location[2] + c.origin[2]];
        cam = coords::transform_point(cam, &c);
    }
    let hf = b.high_frequency_points.max(0) as usize;
    // SheetsUsed (transient): zero for the copy of an Emitter's emitter created in game
    // (InitProperties, Initialize not called again); the sheet loop then draws nothing.
    let sheets = s.sheets_used.max(0);
    let vpp = hf as u32 * 2 * sheets as u32;
    let mut n = 0u32;
    for pi in 0..s.active_particles.max(0) as usize {
        let Some(q) = s.particles.get(pi) else { break };
        if q.flags & 1 == 0 {
            continue;
        }
        let base = hf * pi;
        if hf == 0 || base + hf > s.hf_points.len() {
            continue;
        }
        for sheet in 0..sheets {
            let p0 = s.hf_points[base];
            let p1 = s.hf_points.get(base + 1).copied().unwrap_or(p0);
            let mut prev = [(p0[0] + p0[0]) - p1[0], (p0[1] + p0[1]) - p1[1], (p0[2] + p0[2]) - p1[2]];
            for j in 0..hf {
                let pt = s.hf_points[base + j];
                let d = [pt[0] - prev[0], pt[1] - prev[1], pt[2] - prev[2]];
                let a = [prev[0] - cam[0], prev[1] - cam[1], prev[2] - cam[2]];
                let z = a[1] * d[0] - a[0] * d[1];
                let y = a[0] * d[2] - a[2] * d[0];
                let x = a[2] * d[1] - a[1] * d[2];
                // (X² + Y²) + Z².
                let lsq = x * x + y * y + z * z;
                let mut side = [x, y, z];
                if 1e-8 <= lsq as f64 {
                    let k = 1.0 / lsq.sqrt();
                    side = [side[0] * k, side[1] * k, side[2] * k];
                }
                if sheet != 0 {
                    side = rotate_angle_axis(side, (0x8000 / sheets) * sheet, normalize(d));
                }
                let w = q.size[0];
                side = [side[0] * w, side[1] * w, side[2] * w];
                let u = b.texture_u_scale * pt[3];
                vertices.push(SpriteVertex { position: [side[0] + pt[0], side[1] + pt[1], side[2] + pt[2]], color: q.color, uv: [u, 0.0] });
                vertices.push(SpriteVertex { position: [pt[0] - side[0], pt[1] - side[1], pt[2] - side[2]], color: q.color, uv: [u, b.texture_v_scale] });
                prev = [pt[0], pt[1], pt[2]];
            }
            for j in 0..hf.saturating_sub(1) as u32 {
                let i = vpp * n + (hf as u32 * sheet as u32 + j) * 2;
                indices.extend([i, i + 1, i + 2, i + 1, i + 3, i + 2]);
            }
        }
        n += 1;
    }
    (vertices, indices)
}

/// USpriteEmitter::RenderParticles and UBeamEmitter::RenderParticles,
/// relative CoordinateSystem: SetTransform(TT_LocalToWorld, FRotationMatrix(Owner.Rotation) ×
/// FTranslationMatrix(Owner.Location)) before drawing
/// FillVertexBuffer's local vertices; here the vertices are converted to world space.
pub fn relative_to_world(p: &EmitterParams, owner: &EmitterOwner, vertices: &mut [SpriteVertex]) {
    if p.coordinate_system != 1 {
        return;
    }
    let m = crate::coords::rotation_translation_matrix(owner.rotation, owner.location);
    for v in vertices {
        v.position = crate::coords::transform_fvector(&m, v.position);
    }
}

/// Change of frame of a vector (UseRotationFrom).
type Transform = Box<dyn Fn([f32; 3]) -> [f32; 3]>;

/// What an emitter reads from another emitter of the same actor (Add…FromOtherEmitter).
#[derive(Debug, Clone, Default)]
pub struct OtherEmitter {
    pub active_particles: i32,
    pub locations: Vec<[f32; 3]>,
    pub velocities: Vec<[f32; 3]>,
}

impl OtherEmitter {
    pub fn of(s: &EmitterState) -> Self {
        OtherEmitter {
            active_particles: s.active_particles,
            locations: s.particles.iter().map(|q| q.location).collect(),
            velocities: s.particles.iter().map(|q| q.velocity).collect(),
        }
    }
}

/// AEmitter::Tick for an actor and its emitters. Returns true if the actor must be destroyed
/// (AutoDestroy, all its emitters off).
/// Map trace for particle collisions: (end, start, extent) → (contact,
/// normal) if the trace hits something.
pub type WorldTrace<'a> = dyn FnMut([f32; 3], [f32; 3], [f32; 3]) -> Option<([f32; 3], [f32; 3])> + 'a;

pub fn tick_emitter(owner: &mut EmitterOwner, emitters: &mut [(Box<EmitterParams>, Box<EmitterState>)], dt: f32, r: &mut CRand, trace: &mut WorldTrace) -> bool {
    let mut all_dead = true;
    let mut none_active = true;
    // The other emitters are only read by Add…FromOtherEmitter (its own or a beam's branch
    // emitter, which is also in the list): with none in the list, the copy is unnecessary.
    let read_count = emitters.iter().any(|(p, _)| p.add_location_from_other_emitter >= 0 || p.add_velocity_from_other_emitter >= 0);
    for e in 0..emitters.len() {
        let others: Vec<OtherEmitter> = if read_count { emitters.iter().map(|(_, s)| OtherEmitter::of(s)).collect() } else { Vec::new() };
        let branch = lend_branch(emitters, e);
        tick_one(e, owner, &mut emitters[e], &others, dt, r, &mut all_dead, &mut none_active, trace);
        give_back_branch(emitters, e, branch);
    }
    finish_tick(owner, emitters, dt, r, all_dead, none_active)
}

/// Branch emitter of a beam (BranchEmitter, clamped), taken out of the list and handed to
/// the beam's state.
pub fn lend_branch(emitters: &mut [(Box<EmitterParams>, Box<EmitterState>)], e: usize) -> Option<usize> {
    let EmitterKind::Beam(b) = &emitters[e].0.kind else { return None };
    if !b.use_branching || b.branch_emitter < 0 {
        return None;
    }
    let t = (b.branch_emitter as usize).min(emitters.len() - 1);
    if t == e {
        return None;
    }
    let target = std::mem::take(&mut emitters[t]);
    emitters[e].1.branch_target = Some(Box::new(target));
    Some(t)
}

/// Returns the emitter lent by `lend_branch`.
pub fn give_back_branch(emitters: &mut [(Box<EmitterParams>, Box<EmitterState>)], e: usize, branch: Option<usize>) {
    if let Some(t) = branch {
        if let Some(target) = emitters[e].1.branch_target.take() {
            emitters[t] = *target;
        }
    }
}

fn tick_one(
    index: usize,
    owner: &EmitterOwner,
    emitter: &mut (Box<EmitterParams>, Box<EmitterState>),
    others: &[OtherEmitter],
    dt: f32,
    r: &mut CRand,
    all_dead: &mut bool,
    none_active: &mut bool,
    trace: &mut WorldTrace,
) {
    let (p, s) = emitter;
    if p.disabled || s.disabled {
        return;
    }
    *none_active = false;
    if !s.initialized {
        do_initialize(index, s, p, r);
    }
    s.real_disable_fogging = p.disable_fogging || owner.disable_fogging;
    if s.initial_delay != 0.0 {
        s.initial_delay -= dt;
        if 0.0 < s.initial_delay {
            return;
        }
        s.initial_delay = 0.0;
    }
    s.global_offset = owner.global_offset;
    if !s.warmed_up && p.warmup_ticks_per_second != 0.0 && p.relative_warmup_time != 0.0 {
        let span = (p.lifetime_range.max + p.lifetime_range.min) * p.relative_warmup_time * 0.5;
        let step = 1.0 / p.warmup_ticks_per_second;
        let mut k = 0;
        while k < (span * p.warmup_ticks_per_second) as i32 {
            do_update(index, s, p, others, owner, step, r, trace);
            k += 1;
        }
        s.warmed_up = true;
    }
    if s.all_particles_dead {
        if !p.auto_reset || owner.auto_reset {
            if p.auto_destroy && !owner.auto_destroy {
                s.disabled = true;
            }
            return;
        }
        // !(x > 0) as in AEmitter::Tick's machine code: a NaN also restarts.
        if !(s.time_till_reset - dt > 0.0) {
            s.time_till_reset = p.auto_reset_time_range.get(r);
            do_reset(index, s, p, r);
        } else {
            s.time_till_reset -= dt;
        }
    } else {
        if s.inactive {
            s.inactive_time += dt;
        } else {
            s.inactive_time = 0.0;
        }
        if p.seconds_before_inactive == 0.0 || !(s.inactive_time > p.seconds_before_inactive) {
            do_update(index, s, p, others, owner, dt, r, trace);
        }
        s.inactive = true;
        *all_dead = false;
    }
}

/// UParticleEmitter::Initialize / UpdateParticles / Reset (vtable +0x6C, +0x84, +0x74) called by AEmitter::Tick; in tests
/// they can be replaced to compare the sequence of calls.
fn do_initialize(index: usize, s: &mut EmitterState, p: &EmitterParams, r: &mut CRand) {
    let _ = index;
    s.initialize(p, r);
}

#[allow(clippy::too_many_arguments)]
fn do_update(index: usize, s: &mut EmitterState, p: &EmitterParams, others: &[OtherEmitter], owner: &EmitterOwner, dt: f32, r: &mut CRand, trace: &mut WorldTrace) {
    let _ = index;
    s.update_particles(p, others, owner, dt, r, trace);
}

fn do_reset(index: usize, s: &mut EmitterState, p: &EmitterParams, r: &mut CRand) {
    let _ = index;
    s.reset(p, r);
}

fn finish_tick(owner: &mut EmitterOwner, emitters: &mut [(Box<EmitterParams>, Box<EmitterState>)], dt: f32, r: &mut CRand, all_dead: bool, none_active: bool) -> bool {
    if all_dead {
        if !none_active && !emitters.is_empty() && owner.auto_destroy {
            return true;
        }
        if owner.auto_reset && !owner.auto_destroy {
            let t = owner.time_till_reset - dt;
            if !(t > 0.0) {
                owner.global_offset = owner.global_offset_range.get(r);
                owner.time_till_reset = owner.time_till_reset_range.get(r);
                for (index, (p, s)) in emitters.iter_mut().enumerate() {
                    do_reset(index, s, p, r);
                }
            } else {
                owner.time_till_reset = t;
            }
        }
    }
    false
}

/// FSpriteParticleVertex vertex: position, colour (B, G, R, A), U, V.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpriteVertex {
    pub position: [f32; 3],
    pub color: [u8; 4],
    pub uv: [f32; 2],
}

/// Camera frame read by FillVertexBuffer (FLevelSceneNode: position and axes of
/// CameraToWorld: right, up, forward).
#[derive(Debug, Clone, Copy)]
pub struct ViewFrame {
    pub location: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if (sq as f64) < 1e-8 {
        return v;
    }
    let s = 1.0 / sq.sqrt();
    [v[0] * s, v[1] * s, v[2] * s]
}

/// USpriteEmitter::FillVertexBuffer: four vertices per live particle (corners (0, 0),
/// (1, 0), (0, 1), (1, 1); triangles i, i+1, i+2 and i+2, i+1, i+3).
pub fn fill_sprite_vertices(p: &EmitterParams, s: &mut EmitterState, owner: &EmitterOwner, view: &ViewFrame) -> Vec<SpriteVertex> {
    let EmitterKind::Sprite { use_direction_as, projection_normal } = p.kind else { return Vec::new() };
    // PTDU_Normal, PTDU_UpAndNormal, PTDU_RightAndNormal: RealProjectionNormal
    // (+0x2C8) nearly zero (FVector::IsNearlyZero: each |c| < 1e-4 in double, NaN
    // → false) → ProjectionNormal (+0x2BC).SafeNormal()
    // An emitter copied from its template (AActor::InitProperties) has it reset to zero
    // without going through Initialize: H2HDustEmitter, the fist impact (SpriteEmitter(Emitters[0])
    // .ProjectionNormal = vRand() in its PostBeginPlay), would otherwise stay flat and invisible.
    if matches!(use_direction_as, 4..=6) && s.real_projection_normal.iter().all(|&c| (c as f64).abs() < 1e-4) {
        s.real_projection_normal = safe_normal(projection_normal);
    }
    let s = &*s;
    let mut cam = view.location;
    let mut right = view.right;
    let mut down = [-view.up[0], -view.up[1], -view.up[2]];
    let mut forward = view.forward;
    if p.coordinate_system == 1 {
        // Actor frame: C = GMath.UnitCoords / Owner.Rotation,
        // camera axes through TransformVectorBy, origin of C += Owner.Location
        // then position through TransformPointBy.
        use crate::coords;
        let mut c = coords::div_rotator(&coords::UNIT, owner.rotation);
        down = coords::transform_vector(down, &c);
        right = coords::transform_vector(right, &c);
        forward = coords::transform_vector(forward, &c);
        c.origin = [c.origin[0] + owner.location[0], c.origin[1] + owner.location[1], c.origin[2] + owner.location[2]];
        cam = coords::transform_point(cam, &c);
    }
    let mut projection = [0.0f32; 3];
    if matches!(use_direction_as, 4..=6) {
        projection = s.real_projection_normal;
        // UseRotationFrom = PTRS_Actor: TransformVectorBy(GMath.UnitCoords * Owner.Rotation)
        if p.use_rotation_from == 1 {
            projection = crate::coords::transform_vector(projection, &crate::coords::mul_rotator(&crate::coords::UNIT, owner.rotation));
        }
    }
    let (u0, u1) = (p.center_u - 1.0, p.center_u + 1.0);
    let (v0, v1) = (p.center_v - 1.0, p.center_v + 1.0);
    let mut out = Vec::new();
    for q in s.particles.iter().take(s.active_particles.max(0) as usize) {
        if q.flags & 1 == 0 {
            continue;
        }
        let (mut a, mut b, mut axis): ([f32; 3], [f32; 3], [f32; 3]);
        if use_direction_as != 0 || p.spin_particles {
            let dir = [q.location[0] - q.old_location[0], q.location[1] - q.old_location[1], q.location[2] - q.old_location[2]];
            let (mut d, mut side) = ([0.0f32; 3], [0.0f32; 3]);
            match use_direction_as {
                0 => {}
                3 => d = normalize(dir),
                4 => d = projection,
                5 | 6 => {
                    d = projection;
                    side = normalize(cross(d, forward));
                }
                7 | 8 => {
                    d = normalize(dir);
                    side = normalize(cross(d, s.real_projection_normal));
                }
                _ => {
                    d = normalize(dir);
                    let to_cam = [q.location[0] - cam[0], q.location[1] - cam[1], q.location[2] - cam[2]];
                    side = normalize([d[1] * to_cam[2] - d[2] * to_cam[1], d[2] * to_cam[0] - to_cam[2] * d[0], to_cam[1] * d[0] - d[1] * to_cam[0]]);
                }
            }
            axis = forward;
            match use_direction_as {
                0 => {
                    a = down;
                    b = right;
                }
                1 | 5 | 7 => {
                    a = d;
                    b = side;
                }
                2 | 6 | 8 => {
                    a = side;
                    b = d;
                }
                3 | 4 => {
                    // Support axis, |X| then |Y| compared with 0.57
                    let ax = d.map(f32::abs);
                    let helper = if ax[0] > 0.57 {
                        [0.0, 1.0, 0.0]
                    } else if ax[1] > 0.57 {
                        [0.0, 0.0, 1.0]
                    } else {
                        [1.0, 0.0, 0.0]
                    };
                    // 0x103F91xx: A = Size.Y × (D ^ support), then B = Size.X × SafeNormal(D ^ A)
                    // with A already multiplied.
                    let sy = q.size[1];
                    a = cross(d, helper).map(|c| sy * c);
                    b = safe_normal(cross(d, a)).map(|c| q.size[0] * c);
                    axis = d;
                }
                _ => {
                    a = [0.0; 3];
                    b = [0.0; 3];
                }
            }
            if !matches!(use_direction_as, 3 | 4) {
                let sy = q.size[1];
                let sx = q.size[0];
                a = [a[0] * sy, a[1] * sy, a[2] * sy];
                b = [b[0] * sx, b[1] * sx, b[2] * sx];
            }
            if p.spin_particles {
                let angle = if q.spins_per_second[0] >= 0.0 {
                    (fistp(q.time * q.spins_per_second[0] + q.start_spin[0]) as u32 & 0xFFFF) as i32
                } else {
                    (0xFFFF - (fistp(q.start_spin[0] - q.time * q.spins_per_second[0]) as u32 & 0xFFFF)) as i32
                };
                a = rotate_angle_axis(a, angle, axis);
                b = rotate_angle_axis(b, angle, axis);
            }
        } else {
            a = [down[0] * q.size[1], down[1] * q.size[1], down[2] * q.size[1]];
            b = [right[0] * q.size[0], right[1] * q.size[0], right[2] * q.size[0]];
        }
        let corner = |bu: f32, av: f32| [b[0] * bu + q.location[0] + a[0] * av, b[1] * bu + q.location[1] + a[1] * av, b[2] * bu + q.location[2] + a[2] * av];
        out.push(SpriteVertex { position: corner(u0, v0), color: q.color, uv: [0.0, 0.0] });
        out.push(SpriteVertex { position: corner(u1, v0), color: q.color, uv: [1.0, 0.0] });
        out.push(SpriteVertex { position: corner(u0, v1), color: q.color, uv: [0.0, 1.0] });
        out.push(SpriteVertex { position: corner(u1, v1), color: q.color, uv: [1.0, 1.0] });
    }
    // Texture sub-images: by time (first particle for the lifetime, as
    // the engine does), or the sub-image drawn at spawn.
    if p.texture_u_subdivisions != 0 && p.texture_v_subdivisions != 0 {
        let su = 1.0 / p.texture_u_subdivisions as f32;
        let sv = 1.0 / p.texture_v_subdivisions as f32;
        let inv_life = match s.particles.first() {
            Some(f) if f.max_lifetime != 0.0 => 1.0 / f.max_lifetime,
            _ => 1.0,
        };
        let mut k = 0;
        for q in s.particles.iter().take(s.active_particles.max(0) as usize) {
            if q.flags & 1 == 0 {
                continue;
            }
            let (col, row) = if q.subdivision == -1 {
                let t = (q.time * inv_life).clamp(0.0, 1.0);
                let mut sub = if p.use_subdivision_scale {
                    let mut j = 0;
                    while j < p.subdivision_scale.len() && !(t <= p.subdivision_scale[j]) {
                        j += 1;
                    }
                    j as i32
                } else if p.subdivision_end == 0 {
                    fistp((p.texture_v_subdivisions * p.texture_u_subdivisions) as f32 * t - 0.5)
                } else {
                    let n = (p.subdivision_end - p.subdivision_start).max(1);
                    p.subdivision_start + fistp(n as f32 * t - 0.5)
                };
                let last = p.texture_u_subdivisions * p.texture_v_subdivisions - 1;
                sub = sub.clamp(0, last.max(0));
                (sub / p.texture_v_subdivisions, sub % p.texture_v_subdivisions)
            } else {
                (q.subdivision / p.texture_v_subdivisions, q.subdivision % p.texture_v_subdivisions)
            };
            let v_lo = row as f32 * sv;
            let v_hi = (v_lo + sv).min(1.0);
            let u_lo = col as f32 * su;
            let u_hi = (su + u_lo).min(1.0);
            let base = k * 4;
            out[base].uv = [u_lo, v_lo];
            out[base + 1].uv = [u_hi, v_lo];
            out[base + 2].uv = [u_lo, v_hi];
            out[base + 3].uv = [u_hi, v_hi];
            k += 1;
        }
    }
    let mut k = 0;
    for q in s.particles.iter().take(s.active_particles.max(0) as usize) {
        if q.flags & 1 == 0 {
            continue;
        }
        let base = k * 4;
        if p.symmetry_u && q.flags & 8 != 0 {
            let (a, b) = (out[base].uv[0], out[base + 1].uv[0]);
            out[base].uv[0] = b;
            out[base + 1].uv[0] = a;
            out[base + 2].uv[0] = b;
            out[base + 3].uv[0] = a;
        }
        if p.symmetry_v && q.flags & 0x10 != 0 {
            let (a, c) = (out[base].uv[1], out[base + 2].uv[1]);
            out[base].uv[1] = c;
            out[base + 1].uv[1] = c;
            out[base + 2].uv[1] = a;
            out[base + 3].uv[1] = a;
        }
        k += 1;
    }
    out
}

#[cfg(test)]
mod tests_normal {
    use super::*;

    /// Emitter copied from its template: zero RealProjectionNormal, PTDU_Normal (the fists'
    /// H2HDustEmitter). FillVertexBuffer takes ProjectionNormal.SafeNormal(): the quad is not flat.
    #[test]
    fn normal_projection_null_resume() {
        let p = EmitterParams { kind: EmitterKind::Sprite { use_direction_as: 4, projection_normal: [0.0, 0.0, 2.0] }, ..Default::default() };
        let mut s = EmitterState::default();
        s.particles = vec![Particle { flags: 1, size: [10.0, 10.0, 10.0], ..Default::default() }];
        s.active_particles = 1;
        let owner = EmitterOwner::default();
        let view = ViewFrame { location: [0.0, -100.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], forward: [0.0, 1.0, 0.0] };
        let v = fill_sprite_vertices(&p, &mut s, &owner, &view);
        assert_eq!(s.real_projection_normal, [0.0, 0.0, 1.0]);
        assert_eq!(v.len(), 4);
        assert!(v.iter().any(|x| x.position != v[0].position), "flat quad: {v:?}");
    }
}

