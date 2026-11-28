//! Particle emitters in the VM: ParticleEmitter subobjects, AEmitter::Tick and the
//! UParticleEmitter natives. The simulation is Engine.dll's
//! (`openxiii_assets::particles`); here the properties of the VM objects are read and the
//! native state of each emitter is kept.

use crate::NativeField as Field;
use crate::value::Value;
use crate::{ObjId, Vm};
use openxiii_assets::particles::{self as pa, EmitterKind, EmitterOwner, EmitterParams, EmitterState};
use std::collections::HashMap;

thread_local! {
    pub(crate) static CHECK_PARAMS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Native state of the emitters: particles of each UParticleEmitter, AEmitter data.
#[derive(Debug, Default)]
pub struct ParticleSystems {
    /// Tables taken out and put back every frame (FxHash; never iterated).
    pub states: crate::types::HashMap<ObjId, Box<EmitterState>>,
    /// EmitterParams of each sub-emitter with the generation of its properties when computed
    /// (`emitter_params_kept`); replaced on travel with the rest.
    pub(crate) params_kept: crate::types::HashMap<ObjId, (u64, Box<EmitterParams>)>,
    /// EmitterOwner of each drawn Emitter with the generation of its properties when computed
    /// (`emitter_owner_kept`); GlobalOffset and TimeTillReset are read again there on each call.
    owners_kept: crate::types::HashMap<ObjId, (u64, EmitterOwner)>,
    /// Emitters in level order, reused as long as its actor list does not change.
    render_actors: Vec<ObjId>,
    pub(crate) render_actors_valid: std::cell::Cell<bool>,
    /// Temporary list of sub-emitters during rendering, kept between actors and frames.
    render_emitters: Vec<Option<ObjId>>,
    /// AEmitter::Tick: Emitters list (objects, before filtering out destroyed objects) with the
    /// generation of the actor's properties after `instance_emitters` (`emitter_list_kept`).
    lists_kept: crate::types::HashMap<ObjId, (u64, Vec<Option<ObjId>>)>,
    /// AEmitter::Tick: buffers reused from one actor and one frame to the next.
    tick_buffers: TickBuffers,
    /// AEmitter: GlobalOffset and TimeTillReset (set by AEmitter::PostLoad at the first step).
    pub owners: crate::types::HashMap<ObjId, ([f32; 3], f32)>,
    /// UParticleEmitter.Owner: the actor that last made it live.
    pub emitter_owner: crate::types::HashMap<ObjId, ObjId>,
    /// UBeamEmitter.HitActors (UpdateActorHitList), one actor per BeamEndPoint.
    pub beam_hits: HashMap<ObjId, Vec<Option<ObjId>>>,
    /// emitter_params: slots of the properties read, per emitter class, in reading
    /// order ((address of the name, slot); a read out of this order goes through the name again).
    pub(crate) emitter_slots: std::cell::RefCell<HashMap<crate::value::ClassId, std::rc::Rc<std::cell::RefCell<Vec<(usize, usize, Option<usize>)>>>>>,
}

/// Buffers of `emitter_tick` (empty between two calls).
#[derive(Debug, Default)]
struct TickBuffers {
    list_value: Vec<Option<ObjId>>,
    emitters_list: Vec<(Box<EmitterParams>, Box<EmitterState>)>,
    ids: Vec<ObjId>,
    keys_list: Vec<Option<u64>>,
    /// Empty (Default) states that take the place of the states taken out of the table during the Tick:
    /// swapped with them, then given back, without allocating or copying the 256 bytes.
    reserve: Vec<Box<EmitterState>>,
}

fn emitter_kept() -> bool {
    true
}

/// Particles of a UMeshEmitter ready to draw.
#[derive(Debug, Clone)]
pub struct MeshBatch {
    pub static_mesh: String,
    pub use_mesh_blend_mode: bool,
    pub render_two_sided: bool,
    pub draw_style: u8,
    pub ignore_fog: bool,
    /// Position of the actor (zone, FDynamicActor ambience).
    pub location: [f32; 3],
    pub instances: Vec<pa::MeshInstance>,
    /// Vm::draw_list of the emitter: deferred (1, 2 with bSpecialDelayFov) and bIgnoreVignetteAlpha.
    pub delayed: u8,
    pub ignore_vignette: bool,
    /// Drawn Emitter actor (rank in the list of translucent actors, Vm::translucent_order).
    pub actor: ObjId,
}

/// Particles ready to draw (USpriteEmitter::RenderParticles).
#[derive(Debug, Clone)]
pub struct SpriteBatch {
    pub texture: Option<String>,
    pub draw_style: u8,
    pub ignore_fog: bool,
    pub vertices: Vec<pa::SpriteVertex>,
    /// Animation frame chosen by the emitter's rendering (NoSynchroAnim): texture to draw as is.
    pub frame_texture: Option<String>,
    /// Triangles (beams); empty for sprites (i, i+1, i+2, i+2, i+1, i+3 per quad).
    pub indices: Vec<u32>,
    /// Vm::draw_list of the emitter: deferred (1, 2 with bSpecialDelayFov) and bIgnoreVignetteAlpha.
    pub delayed: u8,
    pub ignore_vignette: bool,
    /// Drawn Emitter actor (rank in the list of translucent actors, Vm::translucent_order).
    pub actor: ObjId,
}

fn range(v: &Value) -> pa::Range {
    match v {
        Value::Struct(_, f) if f.len() >= 2 => pa::Range { min: f[0].as_float(), max: f[1].as_float() },
        _ => pa::Range::default(),
    }
}

fn range_vector(v: &Value) -> pa::RangeVector {
    match v {
        Value::Struct(_, f) if f.len() >= 3 => pa::RangeVector { x: range(&f[0]), y: range(&f[1]), z: range(&f[2]) },
        _ => pa::RangeVector::default(),
    }
}

fn plane(v: &Value) -> [f32; 4] {
    match v {
        Value::Struct(_, f) if f.len() >= 4 => [f[0].as_float(), f[1].as_float(), f[2].as_float(), f[3].as_float()],
        _ => [0.0; 4],
    }
}

/// XIII's Color: fields B, G, R, A.
fn color_bgra(v: &Value) -> [u8; 4] {
    match v {
        Value::Struct(_, f) if f.len() == 4 => [f[0].as_byte(), f[1].as_byte(), f[2].as_byte(), f[3].as_byte()],
        _ => [255; 4],
    }
}

fn material_path(v: &Value) -> Option<String> {
    match v {
        Value::Asset(p) if !p.is_empty() => Some(p.clone()),
        _ => None,
    }
}

impl Vm {
    /// Subobject of a script package ("package.Outer.Name") as a VM object, created
    /// once: actors of the same class share the ParticleEmitters of their default
    /// values, as in the engine (copy of the Emitters list at spawn).
    pub(crate) fn materialize_subobject(&mut self, path: &str) -> Option<ObjId> {
        self.script_object(path)
    }

    /// Emitter spawned in game: the ParticleEmitters of its Emitters list become objects.
    pub(crate) fn instance_emitters(&mut self, a: ObjId) {
        // Already objects (the case on every frame): nothing to do, without copying the list.
        if !matches!(self.propr(a, "Emitters"), Value::Array(items) if items.iter().any(|v| matches!(v, Value::Asset(_)))) {
            return;
        }
        let Value::Array(items) = self.prop(a, "Emitters") else { return };
        let mut changed = false;
        let mut out = items.clone();
        for v in out.iter_mut() {
            if let Value::Asset(path) = v {
                if let Some(id) = self.materialize_subobject(&path.clone()) {
                    *v = Value::Object(Some(id));
                    changed = true;
                }
            }
        }
        if changed {
            self.set_prop(a, "Emitters", Value::Array(out));
        }
    }

    /// Emitter built in game (ULevel::SpawnActor, StaticAllocateObject without
    /// RF_NeedLoad: the actor is passed to UObject::InitProperties). The CopyCompleteValues
    /// of the ConstructorLink chain receive the actor; Emitter.Emitters is the only object property
    /// in the packages that carries 0x400000 (UArrayProperty, element by element), and
    /// UObjectProperty::CopyCompleteValue replaces each ParticleEmitter with
    /// StaticConstructObject(its class, the actor's Outer, NAME_None, 0, template = itself): memory copy
    /// of the template, properties and native state (Particles, Initialized…), without PostLoad or
    /// Initialize, hence without any draw. With a SubobjectRoot (the actor), InitProperties then takes
    /// the transient properties (CPF 0x2000) from the class's default values again, then
    /// the ConstructorLink chain copies the template's arrays again: InitialDelay, TimeTillReset,
    /// PPSFraction, RealProjectionNormal, RealExtentMultiplier… start again from zero, Particles stays
    /// the template's (in the original, such an emitter is born without its 0.2 s delay).
    pub(crate) fn construct_emitters(&mut self, a: ObjId) {
        let Value::Array(items) = self.prop(a, "Emitters") else { return };
        let outer = self.objects[a].as_ref().map(|o| o.outer.clone()).unwrap_or_default();
        let outer_prop = self.prop(a, "Outer");
        let mut out = Vec::with_capacity(items.len());
        for v in items {
            let template = match &v {
                Value::Asset(path) => self.materialize_subobject(&path.clone()),
                Value::Object(Some(o)) => Some(*o),
                _ => None,
            }
            .filter(|&t| self.objects.get(t).is_some_and(|o| o.is_some()));
            out.push(match template {
                Some(t) => Value::Object(Some(self.copy_emitter(t, &outer, &outer_prop))),
                None => v,
            });
        }
        self.set_prop(a, "Emitters", Value::Array(out));
    }

    fn copy_emitter(&mut self, t: ObjId, outer: &str, outer_prop: &Value) -> ObjId {
        let mut state = self.emitter_template_state(t);
        // Transient properties: the class's default values (zero); the arrays
        // (Particles, HFPoints, LFPoints) are copied from the template by the ConstructorLink chain.
        state.particle_index = 0;
        state.active_particles = 0;
        state.pps_fraction = 0.0;
        state.real_extent_multiplier = [0.0; 3];
        state.other_index = 0;
        state.initial_delay = 0.0;
        state.global_offset = [0.0; 3];
        state.time_till_reset = 0.0;
        state.inactive_time = 0.0;
        state.real_projection_normal = [0.0; 3];
        state.beam_value_sum = 0.0;
        state.sheets_used = 0;
        let (class, props) = self.objects[t].as_ref().map(|o| (o.class, o.props.clone())).unwrap();
        let name = self.make_unique_name(class, outer);
        self.object_paths.insert(format!("{outer}.{name}").to_lowercase());
        self.add_name(&name);
        self.objects.push(Some(crate::types::Object { class, props, state: None, state_class: None, name, outer: outer.to_string() }));
        let id = self.objects.len() - 1;
        self.set_intrinsics(id);
        if self.prop_slot(id, "Outer").is_ok() {
            self.set_prop(id, "Outer", outer_prop.clone());
        }
        self.particles.states.insert(id, Box::new(state));
        id
    }

    /// Native state of a template: UParticleEmitter::PostLoad initialized it
    /// (Initialize(MaxParticles), Reset, Initialized) when its package was loaded, before the game:
    /// its draws (InitialDelayRange, AutoResetTimeRange) are not on the game's sequence.
    fn emitter_template_state(&mut self, t: ObjId) -> pa::EmitterState {
        if let Some(s) = self.particles.states.get(&t) {
            return (**s).clone();
        }
        let params = self.emitter_params(t);
        let mut s = pa::EmitterState::default();
        s.initialize(&params, &mut pa::CRand(1));
        self.particles.states.insert(t, Box::new(s.clone()));
        s
    }

    /// UParticleEmitter::PostLoad of a ParticleEmitter, at its rank in the
    /// linker's order (load_order.rs): Initialize(MaxParticles) then Reset, drawn on the game's sequence.
    pub(crate) fn particle_emitter_post_load(&mut self, e: ObjId) {
        if self.particles.states.contains_key(&e) {
            return;
        }
        let params = self.emitter_params(e);
        let mut s = pa::EmitterState::default();
        let mut rng = pa::CRand(self.seed);
        s.initialize(&params, &mut rng);
        // Inactive (bit 0x20000000 of +0x34) is not transient: the map keeps it as
        // the editor left it, and neither Initialize nor Reset clears it;
        // only rendering (RenderParticles) does. An emitter disabled from
        // PostBeginPlay (TrigerredEmitter, bInitiallyOn false) keeps it until its Trigger:
        // AEmitter::Tick then adds dt to InactiveTime from its first update
        // (otherwise its SpriteEmitters stopped one frame later than in the original).
        s.inactive = self.propr(e, "Inactive").as_bool();
        self.seed_advanced(rng.0, crate::draw_kind::N, "particles.rs:205");
        self.particles.states.insert(e, Box::new(s));
    }

    /// PostLoad of the objects produced by a load of the emulated linker: Reset of the
    /// ParticleEmitters (class templates, map emitters), AEmitter::PostLoad of the
    /// map's Emitters (`map`: export → object).
    pub fn apply_post_loads(&mut self, events: &[crate::load_order::PostLoad], map: Option<(&str, &HashMap<usize, ObjId>)>) {
        use crate::load_order::PostLoad;
        for ev in events {
            let (linker_name, export, path, emitter) = match ev {
                PostLoad::ParticleEmitter { linker_name, export, path } => (linker_name, *export, path, false),
                PostLoad::Emitter { linker_name, export, path } => (linker_name, *export, path, true),
                PostLoad::Model { linker_name, export, .. } => {
                    self.model_post_load(linker_name, *export);
                    continue;
                }
            };
            let in_map = map.filter(|(name, _)| name.eq_ignore_ascii_case(linker_name)).and_then(|(_, m)| m.get(&export).copied());
            let id = in_map.or_else(|| self.materialize_subobject(path));
            let Some(id) = id.filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) else { continue };
            if emitter {
                self.emitter_post_load(id);
            } else {
                self.particle_emitter_post_load(id);
            }
        }
    }

    /// UModel::PostLoad (loop): real Init
    /// of all the Model records, then the cache is kept until the level is bound.
    fn model_post_load(&mut self, linker_name: &str, export: usize) {
        // load_map adds the current package before applying its event.
        // An old copy with the same name stays loaded: do not reinitialize
        // its records or overwrite its cache instead of the new receiver.
        let model = self.packages.iter().enumerate()
            .rfind(|(_,p)| p.name.eq_ignore_ascii_case(linker_name))
            .and_then(|(pi,lp)| openxiii_assets::model::Model::read(&lp.package,&lp.data,export).ok().map(|m|(pi,m)));
        let Some((pi,model)) = model else { return };
        let brightness = self.level.map_or(1.0,|level|self.propr(level,"Brightness").as_float());
        let time = self.level_time;
        let mut draws = 0u32;
        let mut lights = Vec::with_capacity(model.lights.len());
        // Walks Num(Model.lights), not static_lights.
        // The real Init shares STATE_TYPE5; the real Vm.rand keeps its tape.
        for record in &model.lights {
            lights.push(openxiii_assets::lighting::init_light(record,brightness,time,&mut||{
                draws += 1;
                self.rand()
            }));
        }
        self.scene_model_postloaded((pi,export),lights);
    }

    /// UParticleEmitter::PostLoad of the ParticleEmitters of an Emitter loaded with the
    /// map: Initialize(MaxParticles) then Reset (InitialDelayRange, AutoResetTimeRange through
    /// appFrand); a template from another package got it when that package was loaded.
    pub(crate) fn emitters_post_load(&mut self, a: ObjId, map_package: &str) {
        self.instance_emitters(a);
        let prefix = format!("{}.", map_package.to_lowercase());
        for e in self.emitter_list(a).into_iter().flatten() {
            if self.particles.states.contains_key(&e) {
                continue;
            }
            if !self.path_name(e).to_lowercase().starts_with(&prefix) {
                self.emitter_template_state(e);
                continue;
            }
            let params = self.emitter_params(e);
            let mut s = pa::EmitterState::default();
            let mut rng = pa::CRand(self.seed);
            s.initialize(&params, &mut rng);
            self.seed_advanced(rng.0, crate::draw_kind::N, "particles.rs:248");
            self.particles.states.insert(e, Box::new(s));
        }
    }

    /// Parameters of an emitter; for a UMeshEmitter, the box of its StaticMesh. EmitterParams of sub-emitter
    /// `e`, reused as is if its properties have not been written since they were computed. Inputs of
    /// emitter_params_with: the properties of `e` (generation), its class (fixed), the box of its StaticMesh
    /// (MeshEmitter: primitive_boxes table, which only grows, path read from the properties) and the slots
    /// kept per class (emitter_slots, emptied with the slots: params_kept too). A BeamEmitter also reads
    /// beam_hits and the position of the hit actors (outside the properties): never kept. Nothing from time,
    /// randomness or the level enters EmitterParams. Directly returns the parameters that depend on the
    /// world; the others are in params_kept.
    fn prepare_params_kept(&mut self, e: ObjId) -> Option<EmitterParams> {
        if self.is(e, crate::flags_classes::D::BeamEmitter) {
            return Some(self.emitter_params(e));
        }
        let Some(g) = self.objects.get(e).and_then(|o| o.as_ref()).map(|o| o.props.generation()) else {
            return Some(self.emitter_params(e));
        };
        if self.particles.params_kept.get(&e).is_some_and(|(g0, _)| *g0 == g) {
            return None;
        }
        let params = self.emitter_params(e);
        self.particles.params_kept.insert(e, (g, Box::new(params)));
        None
    }

    /// Boxed parameters: taken out of the table and put back without copying their 680 bytes.
    fn emitter_params_kept(&mut self, e: ObjId) -> (Box<EmitterParams>, Option<u64>) {
        if let Some(params) = self.prepare_params_kept(e) {
            return (Box::new(params), None);
        }
        let (generation, params) = self.particles.params_kept.remove(&e).unwrap();
        (params, Some(generation))
    }

    fn emitter_params(&mut self, e: ObjId) -> EmitterParams {
        let mesh_box = match (self.is_a(e, "MeshEmitter"), self.native_value(e, Field::StaticMesh)) {
            (true, Value::Asset(m)) => self.primitive_box(&m),
            _ => None,
        };
        self.emitter_params_with(e, mesh_box)
    }

    fn emitter_params_with(&self, e: ObjId, mesh_box: Option<([f32; 3], [f32; 3])>) -> EmitterParams {
        // About a hundred properties read by name on each call: their slots are kept per
        // class in reading order. A read only uses the kept slot if the name is
        // the same constant text (same address) at the same rank; otherwise it goes through propr.
        static VOID: Value = Value::Void;
        let object_value = self.objects.get(e).and_then(|o| o.as_ref()).or_else(|| self.freed.get(&e));
        let table = object_value.map(|o| self.particles.emitter_slots.borrow_mut().entry(o.class).or_default().clone());
        let rank = std::cell::Cell::new(0usize);
        let read_from = |n: &'static str| -> &Value {
            let (Some(o), Some(table)) = (object_value, table.as_ref()) else { return self.propr(e, n) };
            let k = rank.get();
            rank.set(k + 1);
            if let Ok(t) = table.try_borrow() {
                if let Some(&(address, length, case)) = t.get(k) {
                    if address == n.as_ptr() as usize && length == n.len() {
                        return case.and_then(|c| o.props.get(c)).unwrap_or(&VOID);
                    }
                    return self.propr(e, n);
                }
            }
            let case = self.prop_slot(e, n).ok();
            if let Ok(mut t) = table.try_borrow_mut() {
                if t.len() == k {
                    t.push((n.as_ptr() as usize, n.len(), case));
                }
            }
            case.and_then(|c| o.props.get(c)).unwrap_or(&VOID)
        };
        let b = |n: &'static str| read_from(n).as_bool();
        let f = |n: &'static str| read_from(n).as_float();
        let i = |n: &'static str| read_from(n).as_int();
        let by = |n: &'static str| read_from(n).as_byte();
        let v = |n: &'static str| read_from(n).as_vector();
        let rg = |n: &'static str| range(read_from(n));
        let rv = |n: &'static str| range_vector(read_from(n));
        // propr: read without copying (arrays and structs used to be copied every frame).
        let list = |n: &'static str| match read_from(n) {
            Value::Array(items) => items.as_slice(),
            _ => &[],
        };
        let kind = if self.is_a(e, "SpriteEmitter") {
            EmitterKind::Sprite { use_direction_as: by("UseDirectionAs"), projection_normal: v("ProjectionNormal") }
        } else if self.is_a(e, "MeshEmitter") {
            EmitterKind::Mesh {
                static_mesh: material_path(&self.native_prop(e, Field::StaticMesh)),
                use_mesh_blend_mode: b("UseMeshBlendMode"),
                render_two_sided: b("RenderTwoSided"),
                scale_axis: by("ScaleAxis"),
                mesh_box,
            }
        } else if self.is_a(e, "BeamEmitter") {
            let hits = self.particles.beam_hits.get(&e);
            let end_points = list("BeamEndPoints")
                .iter()
                .enumerate()
                .map(|(k, x)| {
                    let (offset, weight) = match x {
                        Value::Struct(_, f) if f.len() >= 3 => (range_vector(&f[1]), f[2].as_float()),
                        _ => (pa::RangeVector::default(), 0.0),
                    };
                    let actor_location = hits
                        .and_then(|h| h.get(k).copied().flatten())
                        .filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some()))
                        .map(|a| self.native_prop(a, Field::Location).as_vector());
                    pa::BeamEndPoint { offset, weight, actor_location }
                })
                .collect();
            let scales = |n: &'static str| -> Vec<([f32; 3], f32)> {
                list(n)
                    .iter()
                    .map(|x| match x {
                        Value::Struct(_, f) if f.len() >= 2 => (f[0].as_vector(), f[1].as_float()),
                        _ => ([0.0; 3], 0.0),
                    })
                    .collect()
            };
            // UBeamEmitter::UpdateParticles clamps BranchEmitter to [0, Num - 1] if it is positive.
            let mut branch_emitter = i("BranchEmitter");
            if branch_emitter >= 0 {
                if let Some(&owner) = self.particles.emitter_owner.get(&e) {
                    let n = self.emitter_list(owner).len() as i32;
                    branch_emitter = if n - 1 <= branch_emitter { n - 1 } else { branch_emitter };
                }
            }
            EmitterKind::Beam(Box::new(pa::BeamParams {
                determine_end_point_by: by("DetermineEndPointBy"),
                beam_distance_range: rg("BeamDistanceRange"),
                end_points,
                texture_u_scale: f("BeamTextureUScale"),
                texture_v_scale: f("BeamTextureVScale"),
                rotating_sheets: i("RotatingSheets"),
                low_frequency_points: i("LowFrequencyPoints"),
                high_frequency_points: i("HighFrequencyPoints"),
                low_frequency_noise_range: rv("LowFrequencyNoiseRange"),
                high_frequency_noise_range: rv("HighFrequencyNoiseRange"),
                lf_scale_factors: scales("LFScaleFactors"),
                hf_scale_factors: scales("HFScaleFactors"),
                lf_scale_repeats: f("LFScaleRepeats"),
                hf_scale_repeats: f("HFScaleRepeats"),
                use_high_frequency_scale: b("UseHighFrequencyScale"),
                use_low_frequency_scale: b("UseLowFrequencyScale"),
                noise_determines_end_point: b("NoiseDeterminesEndPoint"),
                use_branching: b("UseBranching"),
                branch_probability: rg("BranchProbability"),
                branch_emitter,
                branch_spawn_amount_range: rg("BranchSpawnAmountRange"),
                linkup_lifetime: b("LinkupLifetime"),
            }))
        } else {
            EmitterKind::Other
        };
        EmitterParams {
            kind,
            acceleration: v("Acceleration"),
            use_collision: b("UseCollision"),
            use_collision_planes: b("UseCollisionPlanes"),
            use_max_collisions: b("UseMaxCollisions"),
            use_spawned_velocity_scale: b("UseSpawnedVelocityScale"),
            use_color_scale: b("UseColorScale"),
            fade_out: b("FadeOut"),
            fade_in: b("FadeIn"),
            respawn_dead_particles: b("RespawnDeadParticles"),
            auto_destroy: b("AutoDestroy"),
            auto_reset: b("AutoReset"),
            disabled: b("Disabled"),
            disable_fogging: b("DisableFogging"),
            spin_particles: b("SpinParticles"),
            damp_rotation: b("DampRotation"),
            use_size_scale: b("UseSizeScale"),
            use_regular_size_scale: b("UseRegularSizeScale"),
            uniform_size: b("UniformSize"),
            automatic_initial_spawning: b("AutomaticInitialSpawning"),
            use_subdivision_scale: b("UseSubdivisionScale"),
            use_random_subdivision: b("UseRandomSubdivision"),
            no_sync_anim: b("NoSynchroAnim"),
            once_texture_anim: b("OnceTextureAnim"),
            symmetry_u: b("SymmetryU"),
            symmetry_v: b("SymmetryV"),
            random_symmetry_u: b("RandomSymmetryU"),
            random_symmetry_v: b("RandomSymmetryV"),
            extent_multiplier: v("ExtentMultiplier"),
            damping_factor_range: rv("DampingFactorRange"),
            collision_planes: list("CollisionPlanes").iter().map(plane).collect(),
            max_collisions: rg("MaxCollisions"),
            spawn_from_other_emitter: i("SpawnFromOtherEmitter"),
            spawn_amount: i("SpawnAmount"),
            spawned_velocity_scale_range: rv("SpawnedVelocityScaleRange"),
            color_scale: list("ColorScale")
                .iter()
                .map(|c| match c {
                    Value::Struct(_, f) if f.len() >= 2 => (f[0].as_float(), color_bgra(&f[1])),
                    _ => (0.0, [255; 4]),
                })
                .collect(),
            color_scale_repeats: f("ColorScaleRepeats"),
            fade_out_factor: plane(read_from("FadeOutFactor")),
            fade_out_start_time: f("FadeOutStartTime"),
            fade_in_factor: plane(read_from("FadeInFactor")),
            fade_in_end_time: f("FadeInEndTime"),
            coordinate_system: by("CoordinateSystem"),
            max_particles: i("MaxParticles"),
            effect_axis: by("EffectAxis"),
            auto_reset_time_range: rg("AutoResetTimeRange"),
            start_location_offset: v("StartLocationOffset"),
            start_location_range: rv("StartLocationRange"),
            add_location_from_other_emitter: i("AddLocationFromOtherEmitter"),
            start_location_shape: by("StartLocationShape"),
            sphere_radius_range: rg("SphereRadiusRange"),
            start_mass_range: rg("StartMassRange"),
            use_rotation_from: by("UseRotationFrom"),
            rotation_offset: read_from("RotationOffset").as_rotator(),
            spin_ccw_or_cw: v("SpinCCWorCW"),
            spins_per_second_range: rv("SpinsPerSecondRange"),
            start_spin_range: rv("StartSpinRange"),
            rotation_damping_factor_range: rv("RotationDampingFactorRange"),
            rotation_normal: v("RotationNormal"),
            size_scale: list("SizeScale")
                .iter()
                .map(|c| match c {
                    Value::Struct(_, f) if f.len() >= 2 => (f[0].as_float(), f[1].as_float()),
                    _ => (0.0, 1.0),
                })
                .collect(),
            size_scale_repeats: f("SizeScaleRepeats"),
            start_size_range: rv("StartSizeRange"),
            center_u: f("CenterU"),
            center_v: f("CenterV"),
            particles_per_second: f("ParticlesPerSecond"),
            initial_particles_per_second: f("InitialParticlesPerSecond"),
            draw_style: by("DrawStyle"),
            texture: material_path(read_from("Texture")),
            texture_u_subdivisions: i("TextureUSubdivisions"),
            texture_v_subdivisions: i("TextureVSubdivisions"),
            subdivision_scale: list("SubdivisionScale").iter().map(|x| x.as_float()).collect(),
            subdivision_start: i("SubdivisionStart"),
            subdivision_end: i("SubdivisionEnd"),
            seconds_before_inactive: f("SecondsBeforeInactive"),
            min_squared_velocity: f("MinSquaredVelocity"),
            initial_time_range: rg("InitialTimeRange"),
            lifetime_range: rg("LifetimeRange"),
            initial_delay_range: rg("InitialDelayRange"),
            start_velocity_range: rv("StartVelocityRange"),
            max_abs_velocity: v("MaxAbsVelocity"),
            velocity_loss_range: rv("VelocityLossRange"),
            add_velocity_from_other_emitter: i("AddVelocityFromOtherEmitter"),
            get_velocity_direction_from: by("GetVelocityDirectionFrom"),
            warmup_ticks_per_second: f("WarmupTicksPerSecond"),
            relative_warmup_time: f("RelativeWarmupTime"),
        }
    }

    /// AEmitter::PostLoad, for the map's emitters at load time:
    /// GlobalOffset drawn in GlobalOffsetRange (Y, X, Z), then TimeTillReset in
    /// TimeTillResetRange, through appFrand.
    pub(crate) fn emitter_post_load(&mut self, a: ObjId) {
        let global_range = range_vector(self.propr(a, "GlobalOffsetRange"));
        let reset_range = range(self.propr(a, "TimeTillResetRange"));
        let mut rng = pa::CRand(self.seed);
        let g = global_range.get(&mut rng);
        let t = reset_range.get(&mut rng);
        self.seed_advanced(rng.0, crate::draw_kind::N, "particles.rs:456");
        self.particles.owners.insert(a, (g, t));
    }

    /// `emitter_owner` reused as long as the generation of the actor's Props does not change: only
    /// its properties go into it, except GlobalOffset and TimeTillReset (outside the properties), read again here.
    fn emitter_owner_kept(&mut self, a: ObjId) -> EmitterOwner {
        let Some(g) = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.props.generation()) else {
            return self.emitter_owner(a);
        };
        if let Some((g0, kept)) = self.particles.owners_kept.get(&a) {
            if *g0 == g {
                let mut owner = kept.clone();
                let (global_offset, time_till_reset) = self.emitter_owner_values(a);
                owner.global_offset = global_offset;
                owner.time_till_reset = time_till_reset;
                return owner;
            }
        }
        let owner = self.emitter_owner(a);
        self.particles.owners_kept.insert(a, (g, owner.clone()));
        owner
    }
    /// GlobalOffset and TimeTillReset of emitter `a` (zero for one spawned in game, then kept).
    /// Lockstep co-op: this machine's drawing of its own view only reads them, it does not keep
    /// an entry the game would make a few steps later (the state stays the same on every machine).
    fn emitter_owner_values(&mut self, a: ObjId) -> ([f32; 3], f32) {
        if self.lockstep.as_ref().is_some_and(|l| l.drawing()) {
            return self.particles.owners.get(&a).copied().unwrap_or(([0.0; 3], 0.0));
        }
        *self.particles.owners.entry(a).or_insert(([0.0; 3], 0.0))
    }

    fn emitter_owner(&mut self, a: ObjId) -> EmitterOwner {
        let global_range = range_vector(self.propr(a, "GlobalOffsetRange"));
        let reset_range = range(self.propr(a, "TimeTillResetRange"));
        // Emitter spawned in game: no PostLoad, GlobalOffset and TimeTillReset (transient)
        // stay at zero.
        let (global_offset, time_till_reset) = self.emitter_owner_values(a);
        EmitterOwner {
            location: self.native_prop(a, Field::Location).as_vector(),
            rotation: self.native_prop(a, Field::Rotation).as_rotator(),
            velocity: self.native_prop(a, Field::Velocity).as_vector(),
            global_offset,
            auto_destroy: self.propr(a, "AutoDestroy").as_bool(),
            auto_reset: self.propr(a, "AutoReset").as_bool(),
            disable_fogging: self.propr(a, "DisableFogging").as_bool(),
            global_offset_range: global_range,
            time_till_reset_range: reset_range,
            time_till_reset,
            ignore_fog: self.propr(a, "bIgnoreFog").as_bool(),
            pre_pivot: self.native_prop(a, Field::PrePivot).as_vector(),
        }
    }

    /// Properties of the map's BeamEmitters (debugging).
    pub fn debug_beams(&self) {
        for a in self.actors.clone() {
            if !self.is_a(a, "Emitter") {
                continue;
            }
            for e in self.emitter_list(a).into_iter().flatten() {
                if !self.is_a(e, "BeamEmitter") {
                    continue;
                }
                println!("beam {} in {} @ {:?}", self.obj_label(e), self.obj_label(a), self.native_value(a, Field::Location));
                for n in [
                    "DetermineEndPointBy",
                    "BeamDistanceRange",
                    "BeamEndPoints",
                    "BeamTextureUScale",
                    "BeamTextureVScale",
                    "RotatingSheets",
                    "LowFrequencyPoints",
                    "HighFrequencyPoints",
                    "LowFrequencyNoiseRange",
                    "HighFrequencyNoiseRange",
                    "UseHighFrequencyScale",
                    "UseLowFrequencyScale",
                    "NoiseDeterminesEndPoint",
                    "UseBranching",
                    "BranchEmitter",
                    "LFScaleFactors",
                    "HFScaleFactors",
                    "CoordinateSystem",
                    "Texture",
                    "DrawStyle",
                    "MaxParticles",
                    "StartSizeRange",
                    "StartVelocityRange",
                    "LifetimeRange",
                    "LinkupLifetime",
                ] {
                    println!("   {n} = {:?}", self.prop(e, n));
                }
            }
        }
    }

    pub(crate) fn emitter_list(&self, a: ObjId) -> Vec<Option<ObjId>> {
        let mut list = Vec::new();
        self.emitter_list_into(a, &mut list);
        list
    }

    fn emitter_list_into(&self, a: ObjId, list: &mut Vec<Option<ObjId>>) {
        list.clear();
        if let Value::Array(items) = self.propr(a, "Emitters") {
            list.extend(items.iter().map(|v| v.as_object().filter(|&e| self.objects.get(e).is_some_and(|o| o.is_some()))));
        }
    }

    /// `emitter_list_into` without modifying anything (particle drawing): the raw list kept by
    /// `emitter_list_kept` is reused if the generation of the actor's properties has not changed
    /// (same Emitters array, so the same objects; only the filter of destroyed objects is redone),
    /// otherwise Emitters is read again by name as before.
    fn emitter_list_read(&self, a: ObjId, list: &mut Vec<Option<ObjId>>) {
        let g = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.props.generation()).filter(|_| emitter_kept());
        if let Some(g) = g {
            if let Some((_, brute)) = self.particles.lists_kept.get(&a).filter(|(g0, _)| *g0 == g) {
                list.clear();
                list.extend(brute.iter().map(|&e| e.filter(|&e| self.objects.get(e).is_some_and(|o| o.is_some()))));
                return;
            }
        }
        self.emitter_list_into(a, list);
    }

    /// `instance_emitters` then `emitter_list_into`, reused as long as the generation of the actor's
    /// properties does not change: Emitters then no longer has a Value::Asset (instance_emitters
    /// replaced them, which changes the generation) and keeps the same objects; only the filter of
    /// destroyed objects is redone.
    fn emitter_list_kept(&mut self, a: ObjId, list: &mut Vec<Option<ObjId>>) {
        let generation = |vm: &Vm| vm.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.props.generation()).filter(|_| emitter_kept());
        if let Some(g) = generation(self) {
            if let Some((_, brute)) = self.particles.lists_kept.get(&a).filter(|(g0, _)| *g0 == g) {
                list.clear();
                list.extend(brute.iter().map(|&e| e.filter(|&e| self.objects.get(e).is_some_and(|o| o.is_some()))));
                return;
            }
        }
        self.instance_emitters(a);
        self.emitter_list_into(a, list);
        if let Some(g) = generation(self) {
            let brute = match self.propr(a, "Emitters") {
                Value::Array(items) => items.iter().map(|v| v.as_object()).collect(),
                _ => Vec::new(),
            };
            self.particles.lists_kept.insert(a, (g, brute));
        }
    }

    /// AEmitter::Tick, after AActor::Tick.
    pub(crate) fn emitter_tick(&mut self, a: ObjId, dt: f32) {
        if self.native_prop(a, Field::DeleteMe).as_bool() {
            return;
        }
        let mut buffers = std::mem::take(&mut self.particles.tick_buffers);
        let mut list = std::mem::take(&mut buffers.list_value);
        self.emitter_list_kept(a, &mut list);
        let mut owner = if emitter_kept() { self.emitter_owner_kept(a) } else { self.emitter_owner(a) };
        let mut emitters = std::mem::take(&mut buffers.emitters_list);
        let mut ids = std::mem::take(&mut buffers.ids);
        let mut keys_list = std::mem::take(&mut buffers.keys_list);
        // A sub-emitter present twice in the list: previous path (a new default state in
        // its place), so that the pool never receives a state already handled.
        let duplicates = list.iter().flatten().count() > 64 || list.iter().flatten().enumerate().any(|(i, e)| list.iter().flatten().take(i).any(|x| x == e));
        // Bit k: an empty state from the pool was put in place of the k-th sub-emitter.
        let mut poses = 0u64;
        for (k, e) in list.iter().flatten().enumerate() {
            if owner_pose_at_tick(self.pbf(*e, Field::EmitterDisabled)) {
                self.particles.emitter_owner.insert(*e, a);
            }
            self.update_actor_hit_list(*e);
            let (params, key_tuple) = self.emitter_params_kept(*e);
            // Taken out of the table without removing the entry (put back after the Tick): an empty state
            // from the pool holds its place (like the default state that mem::take left).
            let mut state = match self.particles.states.get_mut(e) {
                Some(s) if !duplicates => {
                    poses |= 1 << k;
                    std::mem::replace(s, buffers.reserve.pop().unwrap_or_default())
                }
                Some(s) => std::mem::take(s),
                None => Box::default(),
            };
            state.beam_branch_emitter = None;
            emitters.push((params, state));
            ids.push(*e);
            keys_list.push(key_tuple);
        }
        // appFrand / appRand: rand() of the game thread, the script's sequence.
        let mut rng = pa::CRand(self.seed);
        let drawn = pa::DRAWS.load(std::sync::atomic::Ordering::Relaxed);
        // UParticleEmitter::UpdateParticles: Owner->XLevel->SingleLineCheck(Hit, Owner, Location,
        // OldLocation, TRACE_World, extent); contact if the trace does not go through.
        let destroy = {
            let vm: &Vm = self;
            let mut trace = |end: [f32; 3], start: [f32; 3], extent: [f32; 3]| {
                let mut hit = crate::physics::CheckResult::new(1.0);
                vm.single_line_check(&mut hit, Some(a), end, start, 0x86, extent);
                if hit.time < 1.0 {
                }
                (hit.time < 1.0).then_some((hit.location, hit.normal))
            };
            pa::tick_emitter(&mut owner, &mut emitters, dt, &mut rng, &mut trace)
        };
        let k = String::new();
        self.seed_advanced(rng.0, crate::draw_kind::N, &k);
        if self.rand_counts.is_some() {
            let n = pa::DRAWS.load(std::sync::atomic::Ordering::Relaxed) - drawn;
            if n > 0 {
                let key = format!("particles {}", self.obj_label(a));
                *self.rand_counts.as_mut().unwrap().entry(key).or_default() += n;
            }
        }
        self.particles.owners.insert(a, (owner.global_offset, owner.time_till_reset));
        for (k, ((e, (params, mut state)), key_tuple)) in ids.drain(..).zip(emitters.drain(..)).zip(keys_list.drain(..)).enumerate() {
            // Parameters put back with the generation read before they were computed: a write made since
            // (BranchEmitter, Disabled below, script) changes the generation, they will be recomputed.
            if let Some(g) = key_tuple {
                self.particles.params_kept.insert(e, (g, params));
            }
            // UBeamEmitter::UpdateParticles also writes property +0x340.
            if let Some(branch) = state.beam_branch_emitter.take() {
                self.set_prop(e, "BranchEmitter", Value::Int(branch));
            }
            if state.disabled {
                self.set_prop(e, "Disabled", Value::Bool(true));
            }
            match self.particles.states.get_mut(&e) {
                // The empty state from the pool (intact: nothing reads or writes the state during the Tick)
                // goes back there.
                Some(s) if poses >> k & 1 != 0 => buffers.reserve.push(std::mem::replace(s, state)),
                Some(s) => *s = state,
                None => {
                    self.particles.states.insert(e, state);
                }
            }
        }
        list.clear();
        (buffers.list_value, buffers.emitters_list, buffers.ids, buffers.keys_list) = (list, emitters, ids, keys_list);
        self.particles.tick_buffers = buffers;
        if destroy && !self.native_prop(a, Field::DeleteMe).as_bool() {
            if let Err(err) = crate::natives::call(self, "Destroy", a, &mut [], &[]) {
                self.warn(format!("AutoDestroy of {}: {err}", self.obj_label(a)));
            }
        }
    }

    /// UBeamEmitter::UpdateActorHitList, redone when HitActors does not have as many
    /// entries as BeamEndPoints: for each end, the first actor of the level whose Tag
    /// is ActorTag.
    fn update_actor_hit_list(&mut self, e: ObjId) {
        if !self.is(e, crate::flags_classes::D::BeamEmitter) {
            return;
        }
        let Value::Array(points) = self.prop(e, "BeamEndPoints") else { return };
        if self.particles.beam_hits.get(&e).is_some_and(|h| h.len() == points.len()) {
            return;
        }
        let tags: Vec<String> = points
            .iter()
            .map(|x| match x {
                Value::Struct(_, f) if !f.is_empty() => f[0].as_name(),
                _ => String::new(),
            })
            .collect();
        let hits = tags
            .iter()
            .map(|t| {
                self.actors.iter().copied().find(|&x| self.objects.get(x).is_some_and(|o| o.is_some()) && self.propr(x, "Tag").as_name().eq_ignore_ascii_case(t))
            })
            .collect();
        self.particles.beam_hits.insert(e, hits);
    }

    /// UParticleEmitter::execSpawnParticle.
    pub(crate) fn emitter_spawn_particle(&mut self, e: ObjId, count: i32) {
        let Some(&a) = self.particles.emitter_owner.get(&e) else { return };
        if self.objects.get(a).is_none_or(|o| o.is_none()) {
            return;
        }
        let params = self.emitter_params(e);
        let owner = self.emitter_owner(a);
        let list = self.emitter_list(a);
        let others: Vec<pa::OtherEmitter> =
            list.iter().map(|x| x.and_then(|x| self.particles.states.get(&x)).map(|s| pa::OtherEmitter::of(s)).unwrap_or_default()).collect();
        // appFrand / appRand: rand() of the game thread, the script's sequence.
        let mut rng = pa::CRand(self.seed);
        let branch = match &params.kind {
            EmitterKind::Beam(b) if b.use_branching && b.branch_emitter >= 0 => list.get(b.branch_emitter as usize).copied().flatten().filter(|&t| t != e),
            _ => None,
        };
        let lent = branch.and_then(|t| self.particles.states.remove(&t).map(|s| (t, s)));
        let lent_params = lent.as_ref().map(|(t, _)| self.emitter_params(*t));
        if let Some(state) = self.particles.states.get_mut(&e) {
            if let (Some((_, s)), Some(p)) = (lent.as_ref(), lent_params.as_ref()) {
                state.branch_target = Some(Box::new((Box::new(p.clone()), s.clone())));
            }
            state.spawn_now(&params, &others, &owner, count, &mut rng);
            if let (Some((t, _)), Some(target)) = (lent.as_ref(), state.branch_target.take()) {
                self.particles.states.insert(*t, target.1);
            }
        } else if let Some((t, s)) = lent {
            self.particles.states.insert(t, s);
        }
        self.seed_advanced(rng.0, crate::draw_kind::N, "particles.rs:730");
    }

    /// UParticleEmitter::execSetMaxParticles: MaxParticles then PostEditChange (reinitializes).
    pub(crate) fn emitter_set_max_particles(&mut self, e: ObjId, n: i32) {
        self.set_prop(e, "MaxParticles", Value::Int(n));
        self.particle_emitter_post_edit_change(e);
    }

    /// AEmitter::PostEditChange, after the parent Actor run by the dispatch.
    pub(crate) fn emitter_post_edit_change(&mut self, a: ObjId) {
        for e in self.emitter_list(a).into_iter().flatten() {
            self.particle_emitter_post_edit_change(e);
        }
    }

    pub(crate) fn particle_emitter_post_edit_change(&mut self, e: ObjId) {
        // UParticleEmitter::execSetMaxParticles: MaxParticles then PostEditChange
        // (vtable +0x4C). UParticleEmitter::PostEditChange: if Particles does not have
        // MaxParticles elements or ResetAfterChange, CleanUp (+0x70) then Initialize(MaxParticles)
        // (+0x6C, including Reset and its two appFrand); the subclasses (Sprite,
        // Mesh, Beam) then always redo CleanUp and
        // Initialize. Everything is drawn at once, on the game's sequence.
        let n = self.propr(e, "MaxParticles").as_int();
        self.particles.states.entry(e).or_default();
        let params = self.emitter_params(e);
        let mut rng = pa::CRand(self.seed);
        let reset_after_change = self.pb(e, "ResetAfterChange");
        let state = self.particles.states.get_mut(&e).unwrap();
        if state.particles.len() as i32 != n || reset_after_change {
            state.clean_up();
            state.initialize(&params, &mut rng);
        }
        if self.pb(e, "UseCollisionPlanes") {
            if let Value::Array(mut planes) = self.prop(e, "CollisionPlanes") {
                for plane in &mut planes {
                    if let Value::Struct(_, fields) = plane {
                        if fields.len() >= 4 {
                            let mut normal = [fields[0].as_float(), fields[1].as_float(), fields[2].as_float()];
                            openxiii_assets::coords::normalize(&mut normal);
                            for k in 0..3 { fields[k] = Value::Float(normal[k]); }
                        }
                    }
                }
                self.set_prop(e, "CollisionPlanes", Value::Array(planes));
            }
        }
        if self.is_a(e, "SpriteEmitter") || self.is_a(e, "MeshEmitter") || self.is_a(e, "BeamEmitter") {
            let state = self.particles.states.get_mut(&e).unwrap();
            state.clean_up();
            state.initialize(&params, &mut rng);
        }
        // UBeamEmitter::Initialize (0x1035B7xx-) ends with UpdateActorHitList (vtable +0x8C):
        // HitActors rebuilt at once if the Owner is set (it is at the first render or Tick).
        if self.is_a(e, "BeamEmitter") && self.particles.emitter_owner.contains_key(&e) {
            self.particles.beam_hits.remove(&e);
            self.update_actor_hit_list(e);
        }
        self.seed_advanced(rng.0, crate::draw_kind::N, "particles.rs:768");
    }

    /// AEmitter::Render for all the particles of the visible Emitters (not bHidden): vertices
    /// of the sprites seen from `view`. Rendering clears the Inactive flag of the drawn emitters.
    pub fn particle_sprites(&mut self, view: &pa::ViewFrame) -> Vec<SpriteBatch> {
        self.particle_frame(view).0
    }

    /// Like `particle_sprites`, with the particles of the UMeshEmitters.
    /// AEmitter::Render of an emitter taken by visibility: its active emitters are no longer
    /// inactive (the next Tick resumes UpdateParticles). AEmitter::Render sets
    /// the Owner (+0x260) of each ParticleEmitter and calls RenderParticles (vtable +0x88) of those
    /// without Disabled (+0x34 bit 0x800) when the scene shows particles (SHOW_Particles
    /// 0x1000000, always in game); UParticleEmitter::RenderParticles clears
    /// Inactive (+0x34 bit 0x20000000).
    pub(crate) fn emitter_rendered(&mut self, a: ObjId) {
        let mut emitters = std::mem::take(&mut self.particles.render_emitters);
        self.emitter_list_into(a, &mut emitters);
        for e in emitters.iter().flatten().copied() {
            // AEmitter::Render sets the Owner (+0x260) on every non-null emitter, Disabled included.
            self.particles.emitter_owner.insert(e, a);
            if self.pbf(e, Field::EmitterDisabled) {
                continue;
            }
            if let Some(state) = self.particles.states.get_mut(&e) {
                if !state.disabled {
                    state.inactive = false;
                }
            }
            self.emitter_texture_anim(e);
        }
        self.particles.render_emitters = emitters;
    }

    /// End of USpriteEmitter::RenderParticles, for a drawn emitter (not Disabled): without a texture or
    /// without a living particle (flag 1 of the first ActiveParticles) nothing more. A NoSynchroAnim emitter
    /// (+0x34 bit 0x400000) lends its own animation frame (+0x2A8 AnimCurrent, null: the texture; +0x2AC
    /// Accumulator) to the texture before drawing, LastFrameCount brought back to the previous frame (it advances on every
    /// draw), and takes both fields back afterwards. Outside the editor, if OnceTextureAnim (+0x34 bit
    /// 0x800000) is set and the current frame has no AnimNext (+0x70), the animation is over: all the particles
    /// die, the emitter is Disabled (bit 0x800), ActiveParticles = 0 and the cursor reset to null.
    fn emitter_texture_anim(&mut self, e: ObjId) {
        if !self.is_a(e, "SpriteEmitter") {
            return;
        }
        let Some(state) = self.particles.states.get(&e) else { return };
        if state.disabled {
            return;
        }
        let Value::Asset(texture) = self.propr(e, "Texture") else { return };
        if texture.is_empty() || !state.particles.iter().take(state.active_particles.max(0) as usize).any(|q| q.flags & 1 != 0) {
            return;
        }
        if !self.pb(e, "NoSynchroAnim") {
            if let Some(state) = self.particles.states.get_mut(&e) {
                state.anim_frame = None;
            }
            return;
        }
        let frame = openxiii_assets::material::FrameTime { count: self.engine_frames, delta: self.engine_delta };
        let once = self.pb(e, "OnceTextureAnim");
        let texture = texture.clone();
        let Some(library) = self.library.as_mut() else { return };
        let (cursor, acc) = match &state.anim_cursor {
            Some(c) => (Some(c.clone()), state.anim_accumulator),
            None => (Some(texture.clone()), 0.0),
        };
        // Lockstep co-op: the textures' animation state of the simulation, not the one this
        // machine's drawing (its own view only) also advances: the same on every machine.
        let materials = if self.lockstep.is_some() { &mut **self.lockstep_materials.get_or_insert_with(Default::default) } else { &mut self.canvas_state.materials };
        let (drawn, current, acc) = materials.texture_get_swapped(library, &texture, cursor, acc, frame);
        let finished = once && materials.texture_anim_next(library, current.as_deref().unwrap_or(&texture)).is_none();
        let Some(state) = self.particles.states.get_mut(&e) else { return };
        state.anim_cursor = current;
        state.anim_accumulator = acc;
        state.anim_frame = Some(drawn);
        if finished {
            let n = state.active_particles.max(0) as usize;
            state.last_draw = Some(state.particles.iter().take(n).copied().collect());
            for q in state.particles.iter_mut().take(n) {
                q.flags &= !1;
            }
            state.disabled = true;
            state.active_particles = 0;
            state.anim_cursor = None;
        }
    }

    pub fn particle_frame(&mut self, view: &pa::ViewFrame) -> (Vec<SpriteBatch>, Vec<MeshBatch>) {
        let lockstep_drawing = self.lockstep.as_ref().is_some_and(|l| l.drawing());
        let mut out = Vec::new();
        let mut meshes = Vec::new();
        if !self.particles.render_actors_valid.get() {
            let mut actors = std::mem::take(&mut self.particles.render_actors);
            actors.clear();
            actors.extend(self.actors.iter().copied().filter(|&a| self.is(a, crate::flags_classes::D::Emitter)));
            self.particles.render_actors = actors;
            self.particles.render_actors_valid.set(true);
        }
        let mut emitters = std::mem::take(&mut self.particles.render_emitters);
        for index in 0..self.particles.render_actors.len() {
            let a = self.particles.render_actors[index];
            if self.objects[a].is_none() {
                continue;
            }
            if self.native_prop(a, Field::Hidden).as_bool() {
                continue;
            }
            // Mods build only: a mod's own emitter seen only by its owner, or by everybody but
            // its owner (a flamethrower's jet seen from the eyes by its owner, from the gun by
            // the others), as the game filters the other actors of a view.
            #[cfg(feature = "mods")]
            if let Some(viewer) = self.mod_particle_viewer {
                let only = self.native_prop(a, Field::OnlyOwnerSee).as_bool();
                let no = self.native_prop(a, Field::OwnerNoSee).as_bool();
                if (only || no) && self.objects[a].as_ref().is_some_and(|o| self.class_path(o.class).starts_with("Mod.")) {
                    let owned = self.is_owned_by(a, viewer);
                    if (only && !owned) || (no && owned) {
                        continue;
                    }
                }
            }
            let owner = self.emitter_owner_kept(a);
            let (delayed, ignore_vignette) = self.draw_list(a);
            self.emitter_list_read(a, &mut emitters);
            for e in emitters.iter().flatten().copied() {
                let direct = self.prepare_params_kept(e);
                let params = direct.as_ref().unwrap_or_else(|| &self.particles.params_kept[&e].1);
                if params.disabled {
                    continue;
                }
                let Some(state) = self.particles.states.get_mut(&e) else { continue };
                if state.disabled && state.last_draw.is_none() {
                    continue;
                }
                // Inactive is reset to 0 by AEmitter::Render of the drawn emitters
                // (emitter_rendered, from the rendering of the VM's scene), not by the display:
                // the viewer must not touch the simulation.
                if let EmitterKind::Mesh { static_mesh: Some(mesh), use_mesh_blend_mode, render_two_sided, .. } = &params.kind {
                    let instances = pa::mesh_instances(params, state, &owner);
                    if !instances.is_empty() {
                        meshes.push(MeshBatch {
                            static_mesh: mesh.clone(),
                            use_mesh_blend_mode: *use_mesh_blend_mode,
                            render_two_sided: *render_two_sided,
                            draw_style: params.draw_style,
                            ignore_fog: owner.ignore_fog,
                            location: owner.location,
                            instances,
                            delayed,
                            ignore_vignette,
                            actor: a,
                        });
                    }
                    continue;
                }
                if params.texture.is_some() && matches!(params.kind, EmitterKind::Beam(_)) {
                    let (mut vertices, indices) = pa::fill_beam_vertices(params, state, &owner, view);
                    pa::relative_to_world(params, &owner, &mut vertices);
                    if !indices.is_empty() {
                        out.push(SpriteBatch {
                            texture: params.texture.clone(),
                            draw_style: params.draw_style,
                            ignore_fog: owner.ignore_fog,
                            vertices,
                            frame_texture: None,
                            indices,
                            delayed,
                            ignore_vignette,
                            actor: a,
                        });
                    }
                    continue;
                }
                if params.texture.is_none() || !matches!(params.kind, EmitterKind::Sprite { .. }) {
                    continue;
                }
                // Lockstep co-op: this machine's drawing does not consume it (the steps clear it,
                // lockstep_begin_step), so that the state stays the same on every machine.
                let last = if lockstep_drawing { state.last_draw.clone() } else { state.last_draw.take() };
                let mut vertices = match last {
                    // Last draw of a finished one-shot animation (USpriteEmitter::RenderParticles draws before killing).
                    Some(saved) => {
                        let mut snap = state.clone();
                        snap.active_particles = saved.len() as i32;
                        snap.particles = saved;
                        pa::fill_sprite_vertices(params, &mut snap, &owner, view)
                    }
                    // Lockstep co-op: drawing an emitter for the first time sets its projection
                    // normal (USpriteEmitter::RenderParticles); this machine draws its own view
                    // only, so it fills a copy and the state stays the same on every machine.
                    None if lockstep_drawing && state.real_projection_normal.iter().all(|&c| (c as f64).abs() < 1e-4) => {
                        let mut snap = state.clone();
                        pa::fill_sprite_vertices(params, &mut snap, &owner, view)
                    }
                    None => pa::fill_sprite_vertices(params, state, &owner, view),
                };
                pa::relative_to_world(params, &owner, &mut vertices);
                if vertices.is_empty() {
                    continue;
                }
                out.push(SpriteBatch {
                    texture: params.texture.clone(),
                    draw_style: params.draw_style,
                    ignore_fog: owner.ignore_fog,
                    vertices,
                    frame_texture: if params.no_sync_anim { state.anim_frame.clone() } else { None },
                    indices: Vec::new(),
                    delayed,
                    ignore_vignette,
                    actor: a,
                });
            }
        }
        self.particles.render_emitters = emitters;
        (out, meshes)
    }
}

/// AEmitter::Tick: a Disabled sub-emitter (+0x34 bit 0x800, test) is skipped before
/// its Owner is written (+0x260); only AEmitter::Render sets it then. A
/// BreathSteamEmitter disabled since PostBeginPlay and never drawn receives SpawnParticle without an Owner in
/// the original (no draw); set on every Tick, ours drew 21 times.
pub(crate) fn owner_pose_at_tick(disabled: bool) -> bool {
    !disabled
}

