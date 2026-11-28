//! Game world: a map's actors, state code, latent functions, foreach, timers.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::NativeField as Field;
use crate::types::{Code, Type};
use crate::value::{ClassId, ObjId, Value};
use crate::{Frame, Latent, Place, Result, Vm, VmError};
use openxiii_package::{ObjectRef, Package};
use openxiii_script::bytecode::{Expr, Kind};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

/// Result of UObject::GotoState (EGotoState).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GotoResult {
    NotFound,
    Success,
    /// Another GotoState, made during EndState or BeginState, replaced it.
    Preempted,
}

/// Execution of an actor's state code.
#[derive(Debug, Clone)]
pub struct StateThread {
    pub code: Option<Rc<Code>>,
    pub pc: usize,
    /// GotoState / GotoLabel: code and position replaced; the running code stops and
    /// ProcessState resumes at the new code.
    pub restart: bool,
    /// StateFrame->LatentAction: number of the Poll* native of the current latent action (0 if none).
    pub latent: u16,
}

/// State threads per actor: one slot per object number instead of a BTreeMap (looked up on
/// every actor tick). Same interface and same iteration order (increasing numbers).
#[derive(Debug, Clone, Default)]
pub struct ThreadsState {
    v: Vec<Option<StateThread>>,
}

impl ThreadsState {
    #[inline]
    pub fn get(&self, k: &ObjId) -> Option<&StateThread> {
        self.v.get(*k).and_then(Option::as_ref)
    }
    #[inline]
    pub fn get_mut(&mut self, k: &ObjId) -> Option<&mut StateThread> {
        self.v.get_mut(*k).and_then(Option::as_mut)
    }
    pub fn insert(&mut self, k: ObjId, t: StateThread) -> Option<StateThread> {
        if self.v.len() <= k {
            self.v.resize_with(k + 1, || None);
        }
        self.v[k].replace(t)
    }
    pub fn remove(&mut self, k: &ObjId) -> Option<StateThread> {
        self.v.get_mut(*k).and_then(Option::take)
    }
    pub fn retain(&mut self, mut f: impl FnMut(&ObjId, &mut StateThread) -> bool) {
        for (k, e) in self.v.iter_mut().enumerate() {
            if let Some(t) = e {
                if !f(&k, t) {
                    *e = None;
                }
            }
        }
    }
    /// Numbers of the actors that have a thread, increasing.
    pub fn keys(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.v.iter().enumerate().filter(|(_, e)| e.is_some()).map(|(k, _)| k)
    }
}

impl std::ops::Index<&ObjId> for ThreadsState {
    type Output = StateThread;
    fn index(&self, k: &ObjId) -> &StateThread {
        self.get(k).expect("state thread missing")
    }
}
pub struct MoverShape {
    pub local: Vec<[[f32; 3]; 3]>,
}

impl Vm {
    /// ULevel::Actors of a map (export indices, in order): after the (empty) properties
    /// of the Level object come two 32-bit integers (count, capacity) then the compact
    /// references of the actors.
    pub(crate) fn level_actor_list(&self, pi: usize) -> Option<Vec<usize>> {
        let lp = &self.packages[pi];
        let e = (0..lp.package.exports.len()).find(|&i| lp.package.export_class(i).eq_ignore_ascii_case("Level"))?;
        let mut out = Vec::new();
        for x in openxiii_assets::level::actors_raw(&lp.package, &lp.data, e)? {
            if x > 0 {
                out.push((x - 1) as usize);
            }
        }
        Some(out)
    }

    /// The first two raw references of ULevelBase::Actors (empty slots included).
    fn level_actor_raw_head(&self, pi: usize) -> Vec<i32> {
        let lp = &self.packages[pi];
        let Some(e) = (0..lp.package.exports.len()).find(|&i| lp.package.export_class(i).eq_ignore_ascii_case("Level")) else { return Vec::new() };
        openxiii_assets::level::actors_raw(&lp.package, &lp.data, e).map(|v| v.into_iter().take(2).collect()).unwrap_or_default()
    }

    /// The raw references of the actor array at ULevel+0x1124.
    fn level_light_cases(&self, pi: usize) -> Vec<i32> {
        let lp = &self.packages[pi];
        let Some(e) = (0..lp.package.exports.len()).find(|&i| lp.package.export_class(i).eq_ignore_ascii_case("Level")) else { return Vec::new() };
        openxiii_assets::level::light_actors_raw(&lp.package, &lp.data, e).unwrap_or_default()
    }

    /// ULevel::CleanLightActors (called by UGameEngine::LoadMap outside the editor): DestroyActor(actor, 0, 1) of each non-null
    /// slot of the array whose bHidden is set, in order, then the array is emptied.
    pub(crate) fn clean_light_actors(&mut self) {
        for a in std::mem::take(&mut self.light_actor_cases).into_iter().flatten() {
            if self.objects.get(a).is_some_and(|o| o.is_some()) && self.native_prop(a, Field::Hidden).as_bool() {
                let _ = self.destroy_actor_full(a, false, true);
            }
        }
    }

    /// UGameEngine::LoadMap: if the level's package is not "Entry", every
    /// AActor object of this package (IsIn(GLevel->Outer)) receives RF_EliminateObject (0x400), removed afterwards from those in
    /// GLevel->Actors; CollectGarbage(RF_Native, 0) deletes the marked ones and sets their references to None. Here,
    /// only the actors destroyed during the load (bDeleteMe): our actor list does not hold all
    /// those that the original keeps in Actors (Plage01's AttackPoints).
    /// StaticFindObject no longer finds them (e.g. CineController2's "vf light288": Light288, hidden,
    /// destroyed by CleanLightActors).
    pub(crate) fn remove_actors_outside_level(&mut self, package_value: &str) {
        if package_value.eq_ignore_ascii_case("Entry") {
            return;
        }
        let presents: std::collections::HashSet<ObjId> = self.actors.iter().copied().collect();
        let p = package_value.to_lowercase();
        let of_package = |outer: &str| {
            let o = outer.to_lowercase();
            o == p || o.starts_with(&format!("{p}."))
        };
        let dead: std::collections::HashSet<ObjId> = (0..self.objects.len())
            .filter(|&o| !presents.contains(&o) && self.objects[o].as_ref().is_some_and(|x| of_package(&x.outer)) && self.is_a(o, "Actor") && self.native_prop(o, Field::DeleteMe).as_bool())
            .collect();
        if dead.is_empty() {
            return;
        }
        for o in self.objects.iter_mut().flatten() {
            o.props.iter_mut().for_each(|v| crate::tick::clear_destroyed(v, &dead));
        }
        for &a in &dead {
            if let Some(o) = self.objects[a].take() {
                self.object_paths.remove(&format!("{}.{}", o.outer, o.name).to_lowercase());
                self.freed.insert(a, o);
            }
        }
        self.first_deleted.retain(|a| !dead.contains(a));
    }

    /// Loads a map: its classes, if any, then one VM object per actor.
    pub fn load_map(&mut self, path: &Path) -> std::io::Result<Vec<ObjId>> {
        let _math = self.session.math.enter();
        // Map received from a server (net_file.rs): the library finds it in the cache under its GUID (like
        // appFindPackageFile with the announced GUID, before the game folders); the package keeps the map's name.
        let received = path.file_stem().and_then(|s| s.to_str()).map(str::to_string).and_then(|name_value| self.library.as_ref().and_then(|l| l.file_of(&name_value).cloned()).filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("uxx"))).map(|f| (name_value, f)));
        let (path, name_received): (std::path::PathBuf, Option<String>) = match received {
            Some((name_value, f)) => (f, Some(name_value)),
            None => (path.to_path_buf(), None),
        };
        let path = path.as_path();
        // UGameEngine::LoadMap: "Start LoadMap: %s" (URL.String()).
        // (already written by engine_travel_tick for a travel within the same VM).
        if !std::mem::take(&mut self.start_loadmap_logged) {
            self.audio_start_load_map();
            self.log.push(format!("Log: Start LoadMap: {}", self.url.string()));
        }
        // UGameEngine::LoadMap: GInitRunaway.
        self.runaway = 0;
        let data = std::fs::read(path)?;
        let package = Package::parse(&data).map_err(|e| std::io::Error::other(e.to_string()))?;
        // Package name: that of the file on disk, not the URL's case ("banque01.unr"
        // from Plage01's travel gives the Banque01 package, as in the original XIII.log).
        let typed = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let on_disk = path.parent().and_then(|d| std::fs::read_dir(d).ok()).and_then(|it| {
            it.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|f| f.eq_ignore_ascii_case(typed))
        });
        let name = match name_received {
            Some(n) => n,
            None => on_disk
                .as_deref()
                .map(Path::new)
                .and_then(|f| f.file_stem())
                .and_then(|s| s.to_str())
                .or_else(|| path.file_stem().and_then(|s| s.to_str()))
                // Kept as is: this becomes the level package's name, which appears in path names.
                .unwrap_or("carte")
                .to_string(),
        };
        self.map_file = Some(path.to_path_buf());
        self.name_table = None;
        let pi = self.packages.len();
        self.packages.push(crate::load::LoadedPackage { name, package, data });
        let exports = self.packages[pi].package.exports.len();
        // ULevel::Actors: the map's actors, the LevelInfo first. The package also keeps
        // actors deleted in the editor (bDeleteMe), absent from this list.
        let level_actors = self.level_actor_list(pi);
        let level_set: Option<std::collections::HashSet<usize>> = level_actors.as_ref().map(|l| l.iter().copied().collect());
        let actor_class = self.class("Actor");
        // First pass: creation of the objects whose class is a script class.
        let mut map_objects: HashMap<usize, ObjId> = HashMap::new();
        for i in 0..exports {
            // Editor views (Camera) and CSG brushes are saved with RF_NotForClient |
            // RF_NotForServer (0x300000): their slots in ULevel::Actors are non-empty in the file
            // (serialize_level fingerprint) but the engine does not keep them in game (a Camera, which is a
            // PlayerController, would create a PlayerReplicationInfo and its Timer).
            if self.packages[pi].package.exports[i].flags & 0x300000 == 0x300000 {
                continue;
            }
            let Some(class) = self.class_of_ref(pi, self.packages[pi].package.exports[i].class) else {
                continue;
            };
            if self.packages[pi].package.exports[i].serial_size == 0 {
                continue;
            }

            let (outer, object_name) = {
                let lp = &self.packages[pi];
                let p = &lp.package;
                let full = format!("{}.{}", lp.name, p.object_path(openxiii_package::ObjectRef::Export(i)));
                let outer = full.rsplit_once('.').map_or(String::new(), |(o, _)| o.to_string());
                (outer, p.name(p.exports[i].object_name).to_string())
            };
            let id = self.spawn_in(class, outer, Some(object_name));
            map_objects.insert(i, id);
        }
        // Second pass: properties, with the links between the map's objects.
        let mut actors = Vec::new();
        let mut entries: Vec<(usize, ObjId)> = map_objects.iter().map(|(&e, &o)| (e, o)).collect();
        entries.sort();
        for (export, id) in entries {
            let props = {
                let lp = &self.packages[pi];
                lp.package.object_data(&lp.data, export).map(|o| o.properties).unwrap_or_default()
            };
            let class = self.objects[id].as_ref().unwrap().class;
            for prop in props {
                let pname = self.packages[pi].package.name(prop.name).to_lowercase();
                let Some(&slot) = self.classes[class].prop_index.get(&pname) else {
                    continue;
                };
                let def = self.classes[class].props[slot].clone();
                let value = self.convert_map_value(pi, &def.ty, &prop.value, &map_objects);
                let o = self.objects[id].as_mut().unwrap();
                if def.dim > 1 {
                    if let Value::Array(items) = &mut o.props[slot] {
                        if let Some(item) = items.get_mut(prop.array_index as usize) {
                            *item = value;
                        }
                    }
                } else if prop.array_index == 0 {
                    o.props[slot] = value;
                }
            }
            // Script objects of the packages referenced by the map (default subobjects).
            self.resolve_object_refs(id);
            // AActor::PostLoad: a negative DrawScale becomes
            // -MaterialUSize(Texture) × DrawScale / 128 (FPU 24 bits). AActor::Serialize
            // only makes it negative for LicenseeVer < 0x35 (XIII's maps: 58).
            // Then CheckCollisionType: ColCategory from the collision flags.
            if actor_class.is_some_and(|a| self.is_child_of(class, a)) {
                let scale = self.native_prop(id, Field::DrawScale).as_float();
                if scale < 0.0 {
                    let texture = self.prop(id, "Texture");
                    let usize = self.material_size(&texture).0;
                    self.set_prop(id, "DrawScale", Value::Float((-usize) as f32 * scale * 0.0078125));
                }
                let mut flag_bits = [false; 12];
                for (d, name_value) in flag_bits.iter_mut().zip(FLAGS_COL_CATEGORY) {
                    *d = self.pb(id, name_value);
                }
                self.set_prop(id, "ColCategory", Value::Byte(check_collision_type(flag_bits)));
            }
            // AMover::PostLoad: SavedPos = (−12345, −12345, −12345), SavedRot =
            // (123, 456, 789); the rest (iLink of the brush's polygons) is for the editor.
            if self.is_a(id, "Mover") {
                self.set_prop(id, "SavedPos", Value::vector(-12345.0, -12345.0, -12345.0));
                self.set_prop(id, "SavedRot", Value::rotator(0x7b, 0x1c8, 0x315));
            }
            // ATerrainInfo::Serialize: the sounds and NoiseLoudness at the
            // end of the native part, read after the properties, replace them (Plage00 only
            // has them there).
            if self.is_a(id, "TerrainInfo") {
                if let Some(native) = self.load_terrain_native_props(id, pi, export, &map_objects) {
                    self.terrain_post_load_origin(id, &native);
                }
            }

            // Deleted actors (outside ULevel::Actors) exist as objects, for
            // references, but are not part of the level.
            let in_level = level_set.as_ref().is_none_or(|l| l.contains(&export));
            if actor_class.is_some_and(|a| self.is_child_of(class, a)) && in_level {
                actors.push(id);
                if self.classes[class].name.eq_ignore_ascii_case("LevelInfo") {
                    self.level = Some(id);
                }
            }
        }
        // Actors deleted in the editor (bDeleteMe, outside ULevel::Actors): the references
        // that the level's actors keep to them are None from PostBeginPlay on in the original
        // game (Plage01, TriggerPetitCachottier0: "~ 11 ~ None" for StaticMeshActor79),
        // as UObjectProperty::CleanupDestroyed does for an
        // IsPendingKill actor. The exact path during loading is not identified.
        if let Some(level_set) = level_set.as_ref() {
            let deleted: std::collections::HashSet<ObjId> = map_objects
                .iter()
                .filter(|(e, &id)| !level_set.contains(e) && actor_class.is_some_and(|a| self.objects[id].as_ref().is_some_and(|o| self.is_child_of(o.class, a))) && self.native_prop(id, Field::DeleteMe).as_bool())
                .map(|(_, &id)| id)
                .collect();
            if !deleted.is_empty() {
                fn clear(v: &mut Value, deleted: &std::collections::HashSet<ObjId>) {
                    match v {
                        Value::Object(Some(o)) if deleted.contains(o) => *v = Value::Object(None),
                        Value::Array(items) => items.iter_mut().for_each(|x| clear(x, deleted)),
                        Value::Struct(_, fields) => fields.iter_mut().for_each(|x| clear(x, deleted)),
                        _ => {}
                    }
                }
                for &a in &actors {
                    if let Some(o) = self.objects[a].as_mut() {
                        o.props.iter_mut().for_each(|v| clear(v, &deleted));
                    }
                }
            }
        }
        // ULevel+0x1124: actors read again by ULevel::Serialize for CleanLightActors (LoadMap).
        self.light_actor_cases = self.level_light_cases(pi).into_iter().map(|r| (r > 0).then(|| map_objects.get(&((r - 1) as usize)).copied()).flatten()).collect();
        // Order of ULevel::Actors (serialized with the level), not that of the exports: it is the order
        // of the BeginPlay loops, the Tick and the iterators.
        if let Some(list) = level_actors.as_ref() {
            let present: std::collections::HashSet<ObjId> = actors.iter().copied().collect();
            actors = list.iter().filter_map(|e| map_objects.get(e).copied()).filter(|id| present.contains(id)).collect();
            // Actors(0) and Actors(1) of the file keep their place in LoadMap's sort (sort_actor_list): a null
            // slot (CSG brush or Camera never created) at the head is not replaced by the next actor.
            self.level_head = self
                .level_actor_raw_head(pi)
                .iter()
                .filter(|&&r| r > 0 && map_objects.get(&((r - 1) as usize)).is_some_and(|id| present.contains(id)))
                .count();
        }
        // ULevel::Actors(0) is the map's LevelInfo.
        if let Some(&first) = level_actors.as_ref().and_then(|l| l.first()) {
            if let Some(&id) = map_objects.get(&first) {
                self.level = Some(id);
            }
        }
        // Navigation network: native ReachSpecs of the navigation points.
        self.load_reach_specs(pi, &map_objects);
        // Level is held by the engine for each actor, serialized or not.
        if let Some(level) = self.level {
            for &a in &actors {
                self.set_prop(a, "Level", Value::Object(Some(level)));
                self.set_default_zone(a);
            }
            // Head of the list of navigation points: the one that no other references.
            if let Some(nav) = self.class("NavigationPoint") {
                let points: Vec<ObjId> = actors.iter().copied().filter(|&a| self.objects[a].as_ref().is_some_and(|o| self.is_child_of(o.class, nav))).collect();
                let referenced: std::collections::HashSet<ObjId> = points.iter().filter_map(|&p| self.propr(p, "nextNavigationPoint").as_object()).collect();
                if let Some(&head) = points.iter().find(|p| !referenced.contains(p)) {
                    self.set_prop(level, "NavigationPointList", Value::Object(Some(head)));
                }
            }
        }
        self.actors.extend(actors.iter().copied());
        self.actors_changed();
        // UObject::EndLoad: PostLoad in the linker's creation order (load_order.rs) — Reset
        // of the ParticleEmitters (templates of the classes loaded with the map, the map's emitters)
        // and AEmitter::PostLoad (GlobalOffset, TimeTillReset) interleaved as in the original.
        let linker_order = true;
        if linker_order {
            for &a in &actors {
                if self.is_a(a, "Emitter") {
                    self.instance_emitters(a);
                }
            }
            let level = format!("{}.MyLevel", self.packages[pi].name);
            let events = self.lo_run(|vm, lo| {
                vm.lo_load(lo, &level, Some("Level"));
            });
            let name = self.packages[pi].name.clone();
            self.apply_post_loads(&events, Some((&name, &map_objects)));
        }
        // Then their ParticleEmitters, loaded while reading the actors' properties.
        let map_package = self.packages[pi].name.clone();
        for &a in &actors {
            if self.is_a(a, "Emitter") {
                self.emitters_post_load(a, &map_package);
            }
        }
        // UGameEngine::LoadLevelLocalizedTexts: texts of the actors in "Map.<language>".
        let map_name = self.packages[pi].name.clone();
        self.load_level_localized(&actors, &map_name);
        self.load_brushes(pi, &map_objects, &actors);
        // ULevel::PostLoad: AActor::UpdateLeaves of each actor; then
        // UGameEngine::LoadMap, before InitGame: ULevel::RefreshDisplayable.
        for &a in &actors {
            self.update_leaves(a);
        }
        self.refresh_displayable();
        // UGameEngine::LoadMap (outside the editor): ULevel::CleanLightActors, before UpdateTerrainArrays and RefreshAllDynLight.
        self.clean_light_actors();
        // UGameEngine::LoadMap: ULevel::RefreshAllDynLight (vt 0xF8).
        self.refresh_all_dyn_lights();
        // UGameEngine::LoadMap, outside Entry: the actors of the level's package missing from its
        // Actors (hidden lights destroyed by CleanLightActors) are eliminated by GC2.
        self.remove_actors_outside_level(&map_name);
        // GC2 of UGameEngine::LoadMap, after the level is loaded (log only,
        // gc_log.rs): the packages that only the old map used.
        let unloaded = self.gc_pending_unload.take().unwrap_or_default();
        self.gc_log(&unloaded);
        Ok(actors)
    }

    /// Native TerrainInfo part applied after the properties by load_map.
    pub(crate) fn load_terrain_native_props(&mut self, id: ObjId, pi: usize, export: usize, map_objects: &HashMap<usize, ObjId>) -> Option<openxiii_assets::terrain::TerrainData> {
        let class = self.objects[id].as_ref().unwrap().class;
        let native = {
            let lp = &self.packages[pi];
            let end = lp.package.exports[export].serial_offset as usize + lp.package.exports[export].serial_size as usize;
            lp.package.object_data(&lp.data, export).ok().and_then(|o| openxiii_assets::terrain::read_terrain_native_version_with_visibility(&lp.data[o.end..end], lp.package.header.version, lp.package.header.licensee).ok())
        };
        if let Some((_, t, _, visibility)) = native {
            if let Some(visible) = visibility {
                self.set_prop(id, "QuadVisibilityBitmap", Value::Array(visible.into_iter().map(|word| Value::Int(word as i32)).collect()));
            }
            let licensee = self.packages[pi].package.header.licensee;
            if licensee < 9 {
                self.set_prop(id, "DrawType", Value::Byte(2));
            }
            for (i, (name, r)) in ["FootstepSound", "XIIIFootStepSound", "HitSound", "PNJSndStep", "XIIISndStep"].iter().zip(t.sounds).enumerate() {
                if licensee < 12 || (i >= 3 && licensee < 36) {
                    continue;
                }
                if let Some(&slot) = self.classes[class].prop_index.get(&name.to_lowercase()) {
                    let ty = self.classes[class].props[slot].ty.clone();
                    let v = self.convert_map_value(pi, &ty, &openxiii_package::Value::Object(r), map_objects);
                    self.set_prop(id, name, v);
                }
            }
            if licensee >= 12 {
                self.set_prop(id, "NoiseLoudness", Value::Float(t.noise_loudness));
            }
            Some(t)
        } else {
            None
        }
    }

    /// ATerrainInfo::PostLoad: origin actually
    /// put back at the center of the height map after its native read.
    fn terrain_post_load_origin(&mut self, id: ObjId, t: &openxiii_assets::terrain::TerrainData) {
        let coords: [f32; 12] = [t.to_world.origin, t.to_world.x, t.to_world.y, t.to_world.z].concat().try_into().unwrap();
        let location = self.native_prop(id, Field::Location).as_vector();
        if let Some(c) = openxiii_assets::terrain::post_load_origin(t.heightmap_x, t.heightmap_y, &coords, location) {
            self.log.extend(openxiii_assets::terrain::post_load_origin_log(c));
            self.set_prop(id, "Location", Value::vector(c[0], c[1], c[2]));
        }
    }

    pub(crate) fn convert_map_value(&mut self, pi: usize, ty: &Type, v: &openxiii_package::Value, map: &HashMap<usize, ObjId>) -> Value {
        use openxiii_package::Value as P;
        match (ty, v) {
            (Type::Object(_), P::Object(r)) => match ObjectRef::from(*r) {
                ObjectRef::None => Value::Object(None),
                ObjectRef::Export(e) if map.contains_key(&e) => Value::Object(Some(map[&e])),
                _ => {
                    let p = &self.packages[pi].package;
                    let path = match ObjectRef::from(*r) {
                        ObjectRef::Export(_) => {
                            format!("{}.{}", self.packages[pi].name, p.object_path((*r).into()))
                        }
                        _ => p.object_path((*r).into()),
                    };
                    Value::Asset(path)
                }
            },
            (Type::Array(_), P::Raw(raw)) => match self.convert_raw_array(pi, ty, raw, Some(map)) {
                Some(v) => v,
                None => self.convert_default(pi, ty, v),
            },
            (Type::Struct(_), P::Raw(raw)) => {
                let mut value = self.convert_default(pi, ty, v);
                let mut pos = 0;
                if let Some(decoded) = self.decode_typed(pi, ty, 1, raw, &mut pos, Some(map)) {
                    if pos == raw.len() {
                        value = decoded;
                    }
                }
                value
            }
            _ => self.convert_default(pi, ty, v),
        }
    }

    // ---- State code ----

    /// State code label (UObject::GotoLabel): first state of the chain
    /// (the current state then its UState::SuperField) whose label table contains it; its code
    /// and the label's instruction.
    pub(crate) fn state_label(&self, obj: ObjId, label: &str) -> Option<(Rc<Code>, usize)> {
        let label = label.to_lowercase();
        if label.is_empty() || label == "none" {
            return None;
        }
        let mut state = self.current_state(obj);
        for _ in 0..32 {
            let (ci, sd) = state?;
            if let (Some(off), Some(code)) = (sd.labels.get(&label), &sd.code) {
                return code.at.get(off).map(|&i| (code.clone(), i));
            }
            state = self.parent_state(ci, sd);
        }
        None
    }

    /// Automatic state (GotoState('Auto')): first UState marked `auto` in
    /// TFieldIterator order, fields of the class then of the parent classes.
    pub(crate) fn auto_state(&self, class: ClassId) -> Option<(ClassId, String)> {
        let mut c = Some(class);
        while let Some(ci) = c {
            if let Some(s) = self.classes[ci].states.values().find(|s| s.auto) {
                return Some((ci, s.name.clone()));
            }
            c = self.classes[ci].super_class;
        }
        None
    }

    /// UObject::GotoLabel: the latent action stops and the state code
    /// resumes at the label; with no label found, no more state code.
    pub(crate) fn goto_label(&mut self, obj: ObjId, label: &str) -> bool {
        // StateFrame->LatentAction = 0: the current latent action is abandoned.
        let found = self.state_label(obj, label);
        let ok = found.is_some();
        let (code, pc) = match found {
            Some((c, i)) => (Some(c), i),
            None => (None, 0),
        };
        self.threads.insert(obj, StateThread { code, pc, restart: true, latent: 0 });
        ok
    }

    /// UObject::execGotoState: GotoState if the requested state is not
    /// the current state, then GotoLabel(label, or Begin).
    pub fn goto_state(&mut self, obj: ObjId, state: &str, label: &str) -> Result<()> {
        if self.objects[obj].is_none() {
            return Err(VmError::Unsupported("destroyed object".into()));
        }
        // Mods build only: a mod character's AI controller stays in its allowed states
        // (mod_behaviour.rs).
        #[cfg(feature = "mods")]
        let redirected = self.mod_redirect_state(obj, state);
        #[cfg(feature = "mods")]
        let state = redirected.as_deref().unwrap_or(state);
        let state = if state.eq_ignore_ascii_case("None") { "" } else { state };
        let label = if label.eq_ignore_ascii_case("None") { "" } else { label };
        let show = |n: &str| if n.is_empty() { "None".to_string() } else { n.to_string() };
        let old = self.objects[obj].as_ref().and_then(|o| o.state.clone());
        let current = old.clone().unwrap_or_default();
        let result = if state.eq_ignore_ascii_case(&current) { GotoResult::Success } else { self.goto_state_node(obj, state)? };
        if self.trace_states {
            let who = self.objects[obj].as_ref().map(|o| o.name.clone()).unwrap_or_default();
            let now = self.objects[obj].as_ref().and_then(|o| o.state.clone()).unwrap_or_default();
            let line = format!("STATE {who}: {old:?} -> {now} {label} (t={:.2})", self.time);
            self.log.push(line);
        }
        match result {
            GotoResult::Success => {
                if !self.goto_label(obj, if label.is_empty() { "Begin" } else { label }) && !label.is_empty() {
                    self.native_warn(&format!("GotoState ({} {}): Label not found", show(state), show(label)));
                }
                self.hint_open(obj, state);
            }
            GotoResult::NotFound if !state.is_empty() && !state.eq_ignore_ascii_case("Auto") => {
                self.native_warn(&format!("GotoState ({} {}): State not found", show(state), show(label)));
            }
            _ => {}
        }
        // Mods build only: animation and sound of a mod character's new state.
        #[cfg(feature = "mods")]
        if !self.mods.live.is_empty() {
            self.mod_state_entered(obj, old.as_deref().unwrap_or(""));
        }
        Ok(())
    }

    /// UObject::GotoState: EndState, new state (state code stopped,
    /// probes restored), BeginState. A GotoState made during EndState or BeginState wins.
    fn goto_state_node(&mut self, obj: ObjId, state: &str) -> Result<GotoResult> {
        let class = self.objects[obj].as_ref().map(|o| o.class).ok_or(VmError::Unsupported("destroyed object".into()))?;
        // StateFrame->LatentAction = 0.
        self.set_latent_action(obj, 0);
        let old = self.objects[obj].as_ref().and_then(|o| o.state.clone()).unwrap_or_default();
        let target = if state.eq_ignore_ascii_case("Auto") {
            self.auto_state(class)
        } else if state.is_empty() {
            None
        } else {
            self.find_state(class, state).map(|(c, sd)| (c, sd.name.clone()))
        };
        let new = target.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
        let differs = !new.eq_ignore_ascii_case(&old);
        // EndState and BeginState go through ProcessEvent (vtable +0x14), which does nothing for an object being
        // destroyed (IsPendingKill, vtable +0x38: an actor's bDeleteMe): e.g. a destroyed helicopter
        // crew's fists go to Active without running BeginState.
        let destroyed = self.objects[obj].is_some() && self.is_a(obj, "Actor") && self.native_prop(obj, Field::DeleteMe).as_bool();
        if !old.is_empty() && differs && !destroyed && self.is_probing(obj, "EndState") && !self.in_end_state.contains(&obj) {
            self.state_changed.remove(&obj);
            self.in_end_state.insert(obj);
            let r = match self.find_function(obj, "EndState", false) {
                Some(f) => self.call(f, obj, vec![]).map(|_| ()),
                None => Ok(()),
            };
            self.in_end_state.remove(&obj);
            r?;
            if self.state_changed.contains(&obj) {
                return Ok(GotoResult::Preempted);
            }
        }
        if let Some(o) = self.objects[obj].as_mut() {
            o.state = target.as_ref().map(|(_, n)| n.clone());
            o.state_class = target.as_ref().map(|&(c, _)| c);
        }
        // StateFrame->Code = NULL, ProbeMask recomputed (Disables are forgotten).
        self.threads.insert(obj, StateThread { code: None, pc: 0, restart: true, latent: 0 });
        self.probe_disabled.remove(&obj);
        if new.is_empty() {
            return Ok(GotoResult::NotFound);
        }
        if differs && !destroyed && self.is_probing(obj, "BeginState") {
            self.state_changed.remove(&obj);
            if let Some(f) = self.find_function(obj, "BeginState", false) {
                self.call(f, obj, vec![])?;
            }
            if self.state_changed.contains(&obj) {
                return Ok(GotoResult::Preempted);
            }
        }
        self.state_changed.insert(obj);
        Ok(GotoResult::Success)
    }

    /// Latent action requested by a native that goes through `pending_latent` (execFinishAnim:
    /// LatentAction = EPOLL_FinishAnim, channel already in LatentFloat).
    pub(crate) fn start_latent(&mut self, obj: ObjId, latent: Latent) {
        match latent {
            Latent::FinishAnim => self.set_latent_action(obj, crate::ai_move::EPOLL_FINISH_ANIM),
        }
    }

    /// Advances an actor's state code up to the next wait (AActor::ProcessState): a GotoState made by the state code continues at once in the
    /// new state, up to five state changes per pass.
    pub(crate) fn run_thread(&mut self, obj: ObjId) -> Result<()> {
        let node = |vm: &Vm| vm.objects[obj].as_ref().map(|o| (o.state_class, o.state.clone()));
        let mut old_node = node(self);
        let (mut changes, mut jumps) = (0, 0);
        loop {
            let Some(thread) = self.threads.get_mut(&obj) else {
                return Ok(());
            };
            thread.restart = false;
            let (Some(code), pc) = (thread.code.clone(), thread.pc) else {
                self.threads.remove(&obj);
                return Ok(());
            };
            let mut frame = Frame { this: obj, function: None, locals: Vec::new(), package: code.package, switch: None, iterators: Vec::new() };
            self.in_state_code = true;
            let outer_state = self.state_code_obj.replace(obj);
            let flow = self.run(&code, &mut frame, pc);
            self.state_code_obj = outer_state;
            self.in_state_code = false;
            match flow? {
                crate::Flow::Suspend(pc) => {
                    if let Some(t) = self.threads.get_mut(&obj) {
                        if !t.restart {
                            t.pc = pc;
                        }
                    }
                    return Ok(());
                }
                _ => {
                    let jumped = self.objects[obj].is_some() && self.threads.get(&obj).is_some_and(|t| t.restart);
                    if !jumped {
                        // End of the code (stop) or actor destroyed: StateFrame->Code = NULL.
                        self.threads.remove(&obj);
                        return Ok(());
                    }
                    let now = node(self);
                    if now != old_node {
                        old_node = now;
                        changes += 1;
                        if changes >= 5 {
                            return Ok(());
                        }
                    }
                    jumps += 1;
                    if jumps > 100_000 {
                        return Err(VmError::Unsupported("probable GotoLabel loop".into()));
                    }
                }
            }
        }
    }

    // ---- foreach ----

    /// Elements of a native iterator (AllActors, DynamicActors, RadiusActors…).
    pub(crate) fn iterator_items(&mut self, call: &Expr, frame: &mut Frame) -> Result<(Vec<Value>, Place, Vec<Place>, Option<crate::actor_iter::LazyIter>)> {
        let this = frame.this;
        self.iterator_items_of(call, frame, this)
    }

    /// `this`: the actor of the native iterator, that of the context (`Owner.TouchingActors(…)`,
    /// `Pawn.TouchingActors(…)`, `Other.TraceActors(…)`); the parameters are evaluated in the
    /// caller's frame. Previously, the context was ignored: PotentialClimbWatcher walked its
    /// own contacts (none) and destroyed itself on its first frame (XIII
    /// did not grab LadderVolume3).
    fn iterator_items_of(&mut self, call: &Expr, frame: &mut Frame, this: ObjId) -> Result<(Vec<Value>, Place, Vec<Place>, Option<crate::actor_iter::LazyIter>)> {
        let (name, params) = match &call.kind {
            Kind::NativeFunction(index, params) => {
                let f = *self.native_index.get(index).ok_or_else(|| VmError::UnknownFunction(format!("native({index})")))?;
                (self.functions[f].name.clone(), params)
            }
            Kind::FinalFunction(r, params) => (self.ref_name(frame.package, *r), params),
            Kind::VirtualFunction(n, params) => (self.name_text(frame.package, *n), params),
            // UObject::execContext: the native iterator runs on the context's object;
            // None context: "Accessed None", nothing is walked.
            Kind::Context(object, _, _, inner) => {
                return match self.eval(object, frame)? {
                    Value::Object(Some(o)) => self.iterator_items_of(inner, frame, o),
                    _ => {
                        self.script_warn(inner.offset - 3, "Accessed None");
                        Ok((Vec::new(), Place::Nowhere, Vec::new(), None))
                    }
                };
            }
            _ => return Err(VmError::Unsupported("iterator".into())),
        };
        // AActor::execActorInIterationCategory: (Min, Max, out Actor).
        if name == "ActorInIterationCategory" {
            let mut bytes = [0u8; 2];
            for (i, b) in bytes.iter_mut().enumerate() {
                if let Some(p) = params.get(i) {
                    *b = self.eval(p, frame)?.as_byte();
                }
            }
            let place = match params.get(2) {
                Some(p) => self.place(p, frame)?,
                None => Place::Nowhere,
            };
            let lazy = self.category_iterator(bytes[0], bytes[1]);
            return Ok((Vec::new(), place, Vec::new(), Some(lazy)));
        }
        let class = match params.first() {
            Some(p) => self.eval(p, frame)?.as_class(),
            None => None,
        };
        let place = match params.get(1) {
            Some(p) => self.place(p, frame)?,
            None => Place::Nowhere,
        };
        let matches_class = |vm: &Vm, o: ObjId| class.is_none_or(|c| vm.objects[o].as_ref().is_some_and(|x| vm.is_child_of(x.class, c)));
        if matches!(name.as_str(), "TraceActors") {
            // AActor::execTraceActors: TraceActors(class, out Actor, out HitLoc,
            // out HitNorm, End, optional Start, optional Extent, optional TraceType) walks the
            // contacts of ULevel::MultiLineCheck (TraceType | 0x9F), in order, of the requested class.
            let mut extras = Vec::new();
            for i in [2, 3] {
                extras.push(match params.get(i) {
                    Some(p) => self.place(p, frame)?,
                    None => Place::Nowhere,
                });
            }
            let end = match params.get(4) {
                Some(p) => self.eval(p, frame)?.as_vector(),
                None => [0.0; 3],
            };
            let start = match params.get(5) {
                Some(p) if !matches!(p.kind, Kind::Nothing) => self.eval(p, frame)?.as_vector(),
                _ => self.native_prop(this, Field::Location).as_vector(),
            };
            let extent = match params.get(6) {
                Some(p) if !matches!(p.kind, Kind::Nothing) => self.eval(p, frame)?.as_vector(),
                _ => [0.0; 3],
            };
            let trace_type = match params.get(7) {
                Some(p) if !matches!(p.kind, Kind::Nothing) => self.eval(p, frame)?.as_int() as u32,
                _ => 0,
            };
            let items = self.trace_actors_items(this, class, end, start, extent, trace_type);
            return Ok((items, place, extras, None));
        }
        // Iterators walked during the loop (see `actor_iter`): AllActors,
        // DynamicActors, ChildActors, BasedActors,
        // TouchingActors, RadiusActors, VisibleActors,
        // ZoneActors.
        if matches!(name.as_str(), "AllActors" | "DynamicActors" | "ChildActors" | "BasedActors" | "TouchingActors" | "RadiusActors" | "VisibleActors" | "ZoneActors") {
            let mut tag = String::new();
            let (mut radius, mut loc) = (0.0, self.native_prop(this, Field::Location).as_vector());
            if name == "AllActors" || name == "DynamicActors" {
                if let Some(p) = params.get(2).filter(|p| !matches!(p.kind, Kind::Nothing)) {
                    tag = self.eval(p, frame)?.as_name();
                }
            }
            if name == "RadiusActors" || name == "VisibleActors" {
                if let Some(p) = params.get(2) {
                    radius = self.eval(p, frame)?.as_float();
                }
                if let Some(p) = params.get(3).filter(|p| !matches!(p.kind, Kind::Nothing)) {
                    loc = self.eval(p, frame)?.as_vector();
                }
            }
            let lazy = self.lazy_iterator(&name, this, class, tag, radius, loc);
            return Ok((Vec::new(), place, Vec::new(), lazy));
        }
        let items: Vec<Value> = match name.as_str() {
            // VisibleCollidingActors, VisibleDamageableActors,
            // CollidingActors (see `radius_iter`).
            "VisibleCollidingActors" | "VisibleDamageableActors" | "CollidingActors" => {
                let radius = match params.get(2) {
                    Some(p) => self.eval(p, frame)?.as_float(),
                    None => 0.0,
                };
                let center = match params.get(3) {
                    Some(p) if !matches!(p.kind, Kind::Nothing) => self.eval(p, frame)?.as_vector(),
                    _ => self.native_prop(this, Field::Location).as_vector(),
                };
                let ignore_hidden = match params.get(4) {
                    Some(p) if !matches!(p.kind, Kind::Nothing) => self.eval(p, frame)?.as_bool(),
                    _ => false,
                };
                // Visibility tested on each iteration, after the previous iteration's body (see `visible_radius_ok`).
                if name != "CollidingActors" {
                    let lazy = self.visible_radius_iterator(&name, this, class.or_else(|| self.class("Actor")), radius, center, ignore_hidden);
                    return Ok((Vec::new(), place, Vec::new(), Some(lazy)));
                }
                self.radius_iterator(&name, this, class, radius, center, ignore_hidden).into_iter().map(|o| Value::Object(Some(o))).collect()
            }
            "AllObjects" => (0..self.objects.len()).filter(|&o| self.objects[o].is_some() && matches_class(self, o)).map(|o| Value::Object(Some(o))).collect(),
            other => {
                *self.missing_natives.entry(format!("iterator {other}")).or_default() += 1;
                Vec::new()
            }
        };
        Ok((items, place, Vec::new(), None))
    }

    /// Value of a property by name (Void if missing).
    pub fn prop(&self, obj: ObjId, name: &str) -> Value {
        match self.prop_slot(obj, name) {
            Ok(slot) => self.objects[obj].as_ref().or_else(|| self.freed.get(&obj)).map(|o| o.props[slot].clone()).unwrap_or(Value::Void),
            Err(_) => Value::Void,
        }
    }

    pub fn set_prop(&mut self, obj: ObjId, name: &str, value: Value) {
        if name == "Location" {
        }
        if let Ok(slot) = self.prop_slot(obj, name) {
            if let Some(o) = self.objects[obj].as_mut() {
                o.props[slot] = value;
            }
        }
    }

    // ---- Game loop ----

    /// Unreal stasis: bStasis and no physics (or simple rotation).
    pub fn in_stasis(&self, a: ObjId) -> bool {
        self.native_prop(a, Field::Stasis).as_bool() && matches!(self.native_prop(a, Field::Physics).as_byte(), 0 | 5)
    }

    /// True if the probe event `name` is disabled for this object (Disable or the state's `ignores`).
    pub fn probe_blocked(&self, obj: ObjId, name: &str) -> bool {
        let Some(bit) = probe_bit(name) else {
            return false;
        };
        self.probe_blocked_bit(obj, bit)
    }

    /// probe_blocked for an already known bit.
    pub(crate) fn probe_blocked_bit(&self, obj: ObjId, bit: u32) -> bool {
        if self.probe_disabled.get(&obj).is_some_and(|m| m >> bit & 1 != 0) {
            return true;
        }
        self.current_state(obj).is_some_and(|(_, sd)| sd.ignore_mask >> bit & 1 == 0)
    }

    /// Zone of the actor (Region.Zone): the LevelInfo, default zone, as long as the BSP zones
    /// are not computed.
    pub(crate) fn set_default_zone(&mut self, a: ObjId) {
        let Some(level) = self.level else { return };
        if let Value::Struct(name, mut fields) = self.native_value(a, Field::Region) {
            if fields.first().is_some_and(|z| z.as_object().is_none()) {
                fields[0] = Value::Object(Some(level));
                self.set_prop(a, "Region", Value::Struct(name, fields));
            }
        }
    }

    /// ULevel::DetailChange, called by UGameEngine::Init after SpawnPlayActor and OpenWindow with
    /// RenDev->HighDetailActors ([D3DDrv.D3DRenderDevice] HighDetailActors): LevelInfo.bHighDetailMode (bit 3 of +0x380), then
    /// the Game's DetailChange event (GameInfo: bHighDetail actors destroyed in low detail, LinkToSkybox of each zone).
    pub fn detail_change(&mut self, high: bool) {
        let Some(level) = self.level else { return };
        self.set_prop(level, "bHighDetailMode", Value::Bool(high));
        if let Some(game) = self.propr(level, "Game").as_object() {
            self.event(game, "DetailChange", Vec::new());
        }
    }

    /// HighDetailActors of the render device ([D3DDrv.D3DRenderDevice], true by default as in XIII.ini).
    pub fn high_detail_actors(&mut self) -> bool {
        let v = self.config.as_mut().and_then(|c| c.get_string("D3DDrv.D3DRenderDevice", "HighDetailActors", "System"));
        v.is_none_or(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1"))
    }

    pub fn event_pub(&mut self, obj: ObjId, name: &str, args: Vec<Value>) {
        self.event(obj, name, args);
    }

    /// AActor::ProcessEvent: an actor only receives its events once
    /// Level.bBegunPlay is set (UGameEngine::LoadMap, right before InitGame).
    pub(crate) fn actor_events_open(&self, obj: ObjId) -> bool {
        if !self.is(obj, crate::flags_classes::D::Actor) {
            return true;
        }
        self.level_events_open(obj)
    }

    fn level_events_open(&self, obj: ObjId) -> bool {
        let level = self.native_prop(obj, Field::Level).as_object().or(self.level);
        level.is_none_or(|l| self.native_prop(l, Field::BegunPlay).as_bool())
    }

    /// UObject::ProcessEvent: nothing if the function is a disabled probe
    /// (ProbeMask) or if the object is being destroyed (IsPendingKill, vtable +0x38:
    /// bDeleteMe for an actor).
    fn event(&mut self, obj: ObjId, name: &str, args: Vec<Value>) {
        self.event_prepared(obj, name, || args, crate::tick::IdleEvent::Script);
    }

    pub(crate) fn event_idle_tick(&mut self, obj: ObjId, dt: f32, tick: crate::tick::IdleEvent) {
        self.event_prepared(obj, "Tick", || vec![Value::Float(dt)], tick);
    }

    fn event_prepared(&mut self, obj: ObjId, name: &str, args: impl FnOnce() -> Vec<Value>, tick: crate::tick::IdleEvent) {
        use crate::tick::IdleEvent;
        crate::tally::inc(crate::tally::EVENTS);
        // Event without a function (like ProcessEvent when FindFunction finds nothing): nothing
        // happens, whatever the result of the checks below, which only read.
        // Exit first, without them: about 400 events attempted per frame on Plage01 for ~90 entries
        // in the script. (Tests skip this exit: their stubs observe the attempted events.)
        {
            let without_function = match tick {
                IdleEvent::Missing => true,
                // ProcessEvent starts with IsProbing(Function->GetFName()): for a
                // probe event (names 300 to 363) of an actor (which always has a StateFrame), nothing if
                // the ProbeMask bit is not set, i.e. if the function is only declared
                // (Actor's "event Tick(float DeltaTime);"), even if find_function finds it.
                IdleEvent::Script => {
                    self.objects[obj].is_none()
                        || (self.is(obj, crate::flags_classes::D::Actor) && probe_bit(name).is_some_and(|b| !self.probe_active(obj, b)))
                        || self.find_function(obj, name, false).is_none()
                }
                // Tick whose function only does "return;": Engine.dll's eventTick wrapper
                // only calls ProcessEvent if IsProbing(NAME_Tick) (bit 36 of the
                // ProbeMask, set by the compiler only for a defined function; see the
                // envelope tests). Not probed: no call, nothing written.
                IdleEvent::Returns(_) => self.objects[obj].is_none() || !self.probe_active(obj, 36),
            };
            if without_function {
                return;
            }
        }
        // Tick is bit 36 (FName 336). The plan only concerns actors;
        // Disable/Enable and the level's BeginPlay are read again on each call.
        if self.objects[obj].is_none() || self.native_prop(obj, Field::DeleteMe).as_bool() {
            return;
        }
        let bound = !matches!(tick, IdleEvent::Script);
        let blocked = if bound { self.probe_blocked_bit(obj, 36) } else { self.probe_blocked(obj, name) };
        if blocked || !(if bound { self.level_events_open(obj) } else { self.actor_events_open(obj) }) {
            return;
        }
        match tick {
            IdleEvent::Missing => return,
            IdleEvent::Returns(offset) if self.can_finish_empty_tick() => {
                self.cur_offset = offset;
                self.return_nothing = false;
                return;
            }
            _ => {}
        }
        let args = args();
        if let Some(f) = self.find_function(obj, name, false) {
            // After resolution in the state and the class: an inherited Tick may contain
            // only "return;". A redefinition by the game or a mod keeps its bytecode.
            if name == "Tick" && matches!(args.as_slice(), [Value::Float(_)]) && self.finish_empty_tick(f) {
                return;
            }
            if let Err(e) = self.call(f, obj, args.into_iter().map(Some).collect()) {
                self.warn(format!("{name} on {}: {e}", self.objects[obj].as_ref().map_or("?", |o| &o.name)));
            }
        }
    }

    pub(crate) fn empty_tick_offset(&self, f: crate::types::FuncId) -> Option<u32> {
        use crate::types::{ParamDef, PropDef};
        use openxiii_script::objects::func;
        let d = &self.functions[f];
        let [stmt] = d.code.as_ref()?.statements.as_slice() else { return None };
        if !matches!(&stmt.kind, Kind::Return(value) if matches!(value.kind, Kind::Nothing))
            || d.native != 0 || d.flags & (func::NATIVE | func::SINGULAR | func::NET) != 0 || d.ret.is_some()
            || !matches!(d.locals.as_slice(), [PropDef { ty: Type::Float, dim: 1, .. }])
            || !matches!(d.params.as_slice(), [ParamDef { local: 0, out: false, .. }])
        {
            return None;
        }
        Some(stmt.offset)
    }

    fn can_finish_empty_tick(&self) -> bool {
        // call_inner stays responsible for networking, the recursion limit and traces.
        self.ipdrv.net_driver.is_none() && self.depth <= 250 && self.trace_steps.is_none()

    }

    fn finish_empty_tick(&mut self, f: crate::types::FuncId) -> bool {
        let Some(offset) = self.empty_tick_offset(f) else { return false };
        if !self.can_finish_empty_tick() { return false; }
        self.cur_offset = offset;
        self.return_nothing = false;
        true
    }

    /// InitExecution (vt 0x44) specific to some classes, after AActor::InitExecution (
    /// UObject::InitExecution and checks only). The engine sets fixed values that the scripts read:
    /// - AGameInfo::InitExecution: DummyStuff1 = -326.27 and DummyStuff2 = 0x337,
    ///   read by IAController.Init (soldiers' health and Skill);
    /// - AGenAlerte::InitExecution (XIDPawn.dll): dummy = 0x7D2, read when a soldier
    ///   runs to an alarm.
    pub(crate) fn init_execution(&mut self, id: ObjId) {
        if self.is_a(id, "GameInfo") {
            self.set_prop(id, "DummyStuff1", Value::Float(-326.27));
            self.set_prop(id, "DummyStuff2", Value::Int(0x337));
        } else if self.is_a(id, "GenAlerte") {
            self.set_prop(id, "dummy", Value::Int(0x7d2));
        }
    }

    /// GameInfo class chosen by UGameEngine::LoadMap: the URL's last
    /// "GAME=" option, otherwise LevelInfo.DefaultGameType (XIIIMP.XIIIMPGameInfo for DM maps,
    /// XIIIMPCTFGameInfo, XIIIMPBombGame…), otherwise GameInfo for Entry, otherwise
    /// [Engine.Engine] DefaultGame (DefaultServerGame without a client).
    pub fn game_class_for(&mut self, url: &crate::travel::Url) -> String {
        let mut game = String::new();
        for op in &url.options {
            // Parse(Option, "GAME="): the key is searched anywhere in the option.
            if let Some(at) = op.to_ascii_lowercase().find("game=") {
                game = op[at + 5..].split([' ', '\r', '\n', '\t', ',']).next().unwrap_or("").to_string();
            }
        }
        if !game.is_empty() {
            return game;
        }
        if let Some(level) = self.level {
            let default = self.propr(level, "DefaultGameType").as_name();
            if !default.is_empty() {
                return default;
            }
        }
        let map = url.map.rsplit_once('.').map_or(url.map.as_str(), |(m, _)| m);
        if map.eq_ignore_ascii_case("Entry") {
            return "Engine.GameInfo".into();
        }
        // UGameEngine::LoadMap: "ini:Engine.Engine.DefaultGame" with a UClient,
        // "…DefaultServerGame" without (dedicated server).
        let key = if self.ipdrv.dedicated { "DefaultServerGame" } else { "DefaultGame" };
        self.config.as_mut().and_then(|c| c.get_string("Engine.Engine", key, "System")).unwrap_or_else(|| "XIII.XIIIGameInfo".into())
    }

    /// Creates the GameInfo like the engine: without events (AActor::ProcessEvent before bBegunPlay),
    /// linked to Level.Game; InitGame happens in begin_play.
    pub fn start_game(&mut self, game_class: &str, options: &str) -> Result<ObjId> {
        let _math = self.session.math.enter();
        let class = self.class(game_class).ok_or_else(|| VmError::UnknownClass(game_class.to_string()))?;
        // UGameEngine::LoadMap: "Game class is '%s'" (GetName of the class).
        self.log.push(format!("Log: Game class is '{}'", self.classes[class].name));
        let game = self.spawn(class);
        self.init_execution(game);
        if let Some(level) = self.level {
            self.set_prop(game, "Level", Value::Object(Some(level)));
            self.set_prop(level, "Game", Value::Object(Some(game)));
        }
        // ULevel::SpawnActor: added at the end of ULevel::Actors, after the map's actors
        // (its PostBeginPlay comes after theirs, as in the original XIII.log).
        self.actors.push(game);
        self.actors_changed();
        // UGameEngine::LoadMap: ULevel::Listen after the GameInfo and before InitGame,
        // without a UClient (dedicated server) or if the URL has "Listen" (the local client stays:
        // NM_ListenServer).
        if self.ipdrv.dedicated || self.url.has_option("Listen") {
            self.level_listen();
        }
        // InitGame comes later, in begin_play (after InitExecution and bBegunPlay).
        self.pending_init_game = Some((game, options.to_string()));
        Ok(game)
    }

    /// UGameEngine::LoadMap: LevelInfo.bBegunPlay and bStartup
    /// (+0x380 |= 0x42), then Level.Game.InitGame(Options, Error); then the server's
    /// package map.
    fn load_map_init_game(&mut self) -> Result<()> {
        if let Some(level) = self.level {
            self.set_prop(level, "bBegunPlay", Value::Bool(true));
            self.set_prop(level, "bStartup", Value::Bool(true));
        }
        if let Some((game, options)) = self.pending_init_game.take() {
            if self.alive(game) {
                if let Some(f) = self.find_function(game, "InitGame", false) {
                    self.call(f, game, vec![Some(Value::Str(options)), Some(Value::Str(String::new()))])?;
                }
            }
            // UGameEngine::LoadMap: the server's package map once the GameInfo is ready.
            if self.ipdrv.net_driver.as_ref().is_some_and(|d| d.server_conn.is_none()) {
                self.build_server_master_map();
            }
        }
        Ok(())
    }

    /// APlayerController::LocalPlayerController (vt 0x150): the controller itself
    /// if its Player is not null and derives from UViewport (the "WindowsViewportN" object of
    /// WindowsClient0). A remote client's Player is a UNetConnection: refused.
    pub fn local_player_controller(&self, c: ObjId) -> Option<ObjId> {
        if !self.is_a(c, "PlayerController") {
            return None;
        }
        let player = self.native_prop(c, Field::Player).as_object()?;
        self.objects.get(player)?.as_ref().filter(|o| o.outer == "Transient.WindowsClient0")?;
        Some(c)
    }

    /// Connection of the local player, like ULevel::SpawnPlayActor: Login then PostLogin.
    /// What the engine does for a local player: InteractionMaster and console (Console= in
    /// XIII.ini), a Player object (the game window) linked to the PlayerController, then
    /// InitInputSystem, where the script creates its interactions (XIIIPlayerInteraction…).
    pub(crate) fn attach_player(&mut self, pc: ObjId, viewport: Option<ObjId>) {
        let Some(player_class) = self.class("Player") else {
            return;
        };
        // UWindowsViewport (WinDrv.dll): native "WindowsViewportN" object of WindowsClient0 (the
        // viewport given by SpawnPlayActor, see client.rs, otherwise a new one).
        let player = match viewport {
            Some(v) => v,
            None => {
                let client = "Transient.WindowsClient0";
                let index = self.objects.iter().flatten().filter(|o| o.outer == client).count();
                self.spawn_in(player_class, client.to_string(), Some(format!("WindowsViewport{index}")))
            }
        };
        self.open_viewport_window(player);
        // APlayerController::SetPlayer: the Player's old controller loses it, then
        // Player->OptimizeInputBindings (vt 0x70, UViewport: bindings compiled by
        // the host, input_init) and, for a viewport, InitInputSystem.
        self.set_player(pc, player, true);
    }

    /// APlayerController::SetPlayer: the Player's old controller loses it, Player and Actor point to each other, InitInputSystem for
    /// a viewport (IsA UViewport) and "Possessed PlayerPawn: <full name>" (NAME_Log, 0x2F8). (UPlayer::OptimizeInputBindings, vtable +0x70, is done
    /// by the caller for a viewport.)
    pub(crate) fn set_player(&mut self, pc: ObjId, player: ObjId, viewport: bool) {
        if let Some(old) = self.propr(player, "Actor").as_object().filter(|&o| self.alive(o)) {
            self.set_prop(old, "Player", Value::Object(None));
        }
        self.set_prop(pc, "Player", Value::Object(Some(player)));
        self.set_prop(player, "Actor", Value::Object(Some(pc)));
        if viewport {
            // APlayerController::eventInitInputSystem.
            self.event(pc, "InitInputSystem", vec![]);
        }
        let class = self.objects[pc].as_ref().map(|o| self.classes[o.class].name.clone()).unwrap_or_default();
        self.log.push(format!("Log: Possessed PlayerPawn: {class} {}", self.path_name(pc)));
    }

    /// Window of a viewport: InteractionMaster and console (Console= in [Engine.Engine], created
    /// once), then the window's GUIController (GUIController=, LocalInteractions[0]) and its
    /// InitializeController, like UGameEngine when the viewport is created.
    pub(crate) fn open_viewport_window(&mut self, player: ObjId) {
        let Some(master_class) = self.class("InteractionMaster") else {
            return;
        };
        let config = |vm: &mut Vm, key: &str, default: &str| {
            vm.config.as_mut().and_then(|c| c.get_string("Engine.Engine", key, "System")).unwrap_or_else(|| default.to_string())
        };
        let console_class = config(self, "Console", "XIDInterf.XIIIConsole");
        let gui_class = config(self, "GUIController", "");
        let master = match self.interaction_master {
            Some(m) => m,
            None => {
                let m = self.spawn(master_class);
                self.interaction_master = Some(m);
                self.display_copyright(m);
                if let Some(f) = self.find_function(m, "AddInteraction", false) {
                    if let Ok((console, _)) = self.call(f, m, vec![Some(Value::Str(console_class))]) {
                        self.set_prop(m, "Console", console);
                    }
                }
                m
            }
        };
        self.set_prop(player, "InteractionMaster", Value::Object(Some(master)));
        let console = self.prop(master, "Console");
        self.set_prop(player, "Console", console);
        // UViewport::InitInteraction: AddInteraction("ini:Engine.Engine.GUIController",
        // viewport) then InitializeController; once per window (a reused viewport keeps its own).
        let has_local = matches!(self.prop(player, "LocalInteractions"), Value::Array(ref a) if !a.is_empty());
        if !gui_class.is_empty() && !has_local {
            if let Some(f) = self.find_function(master, "AddInteraction", false) {
                match self.call(f, master, vec![Some(Value::Str(gui_class)), Some(Value::Object(Some(player)))]) {
                    // Like UGameEngine when the window is created: InitializeController.
                    Ok((gui, _)) => {
                        if let Some(gui) = gui.as_object() {
                            // UBaseGUIController::eventInitializeController.
                            self.event(gui, "InitializeController", vec![]);
                            // After a lockstep session: the menu pages kept before it.
                            if let Some(m) = self.lockstep_menus.take() {
                                let given = self.give_saved_menu_stack_to(gui, &m);
                                self.log.push(format!("Log: Network: the menus kept before the session given back ({given})"));
                            }
                        }
                    }
                    Err(e) => self.warn(format!("GUIController: {e}")),
                }
            }
        }
    }

    /// UInteractionMaster::DisplayCopyright, called by UGameEngine::Init
    /// right after the InteractionMaster is created: LocalizeGeneral Engine and
    /// Copyright from Core, then two "" lines (GLog, "Warning:" log); if
    /// GlobalTimeOutArmed, TimeOfLastInput = appSeconds.
    fn display_copyright(&mut self, master: ObjId) {
        for key in ["Engine", "Copyright"] {
            let text = self.localize("General", key, "Core", false);
            self.log.push(format!("Warning: {text}"));
        }
        for _ in 0..2 {
            self.log.push("Warning:  ".to_string());
        }
        if self.propr(master, "GlobalTimeOutArmed").as_bool() {
            self.set_prop(master, "TimeOfLastInput", Value::Float(self.app_seconds as f32));
        }
    }

    /// Console command ("SelectWeapon 1", "Fire", "UnFire"…), like the game's keys
    /// (DefUser.ini): `exec` function of the controller, its pawn, its HUD or its weapon.
    /// Returns false if nobody knows it.
    /// UObject::ScriptConsoleExec: first word = exec function of the object;
    /// each parameter is imported from the rest of the line (UProperty::ImportText); a first
    /// object parameter that the executor can fill receives it. Unreadable mandatory parameter:
    /// error, the command is taken without a call.
    pub(crate) fn script_console_exec(&mut self, obj: ObjId, command: &str, executor: Option<ObjId>) -> bool {
        let chars: Vec<char> = command.chars().collect();
        // ParseToken(&Cmd, buf, 0x43, 1): spaces and tabs skipped, a word or a text in quotes;
        // an empty token: false.
        let Some((name, rest)) = crate::console::parse_token(command) else {
            return false;
        };
        let mut p = chars.len() - rest.chars().count();
        let Some(f) = self.find_function(obj, &name, false) else {
            return false;
        };
        if self.functions[f].flags & openxiii_script::objects::func::EXEC == 0 {
            return false;
        }
        let params = self.functions[f].params.clone();
        let mut args: Vec<Option<Value>> = Vec::with_capacity(params.len());
        for (i, prm) in params.iter().enumerate() {
            let def = self.functions[f].locals.get(prm.local).cloned();
            let Some(def) = def else { break };
            if i == 0 {
                if let (Some(e), Type::Object(cls)) = (executor, &def.ty) {
                    let fits = self.class(cls).is_some_and(|c| self.objects[e].as_ref().is_some_and(|o| self.is_child_of(o.class, c)));
                    if fits {
                        args.push(Some(Value::Object(Some(e))));
                        continue;
                    }
                }
            }
            p = parse_next(&chars, p);
            let mut v = self.zero(&def.ty, 1);
            match self.import_text_named(&def.name, &def.ty, def.enumeration.as_deref(), &chars, p, 1, &mut v) {
                Some(np) => {
                    p = np;
                    args.push(Some(v));
                }
                None if prm.optional => break,
                None => {
                    self.log.push(format!("{name}: bad or missing property '{}'", def.name));
                    return true;
                }
            }
        }
        if let Err(e) = self.call(f, obj, args) {
            self.warn(format!("command {command}: {e}"));
        }
        true
    }

    pub fn login(&mut self, game: ObjId, options: &str) -> Result<Option<ObjId>> {
        self.login_travel(game, "", options, None)
    }

    /// ULevel::SpawnPlayActor: GameInfo.Login(Portal, Options), the player, the travel
    /// inventory (TravelPreAccept, AcceptInventory, TravelPostAccept), then PostLogin.
    pub fn login_travel(&mut self, game: ObjId, portal: &str, options: &str, travel: Option<&str>) -> Result<Option<ObjId>> {
        let viewport = self.engine.client.viewports.first().copied().and_then(|s| self.viewport_object(s));
        let info = travel_info_named(options, travel);
        self.login_viewport(game, portal, options, &info, viewport, 2)
    }

    /// ULevel::SpawnPlayActor for viewport `viewport` (see client.rs): `info` is the level's TravelInfo (player name, ExportTravel text).
    pub(crate) fn login_viewport(&mut self, game: ObjId, portal: &str, options: &str, info: &[(String, String)], viewport: Option<ObjId>, remote_role: u8) -> Result<Option<ObjId>> {
        let test_value: Option<(Option<ObjId>, String, usize)> = None;
        // The viewport's window exists before SpawnPlayActor (UGameEngine::Init):
        // its GUIController is created before Login.
        if let (Some(v), None) = (viewport, &test_value) {
            self.open_viewport_window(v);
        }
        // Actors.Num before Login: a Login that creates no actor does not receive the travel inventory.
        let actors_before = self.actors.len();
        let (pc, error) = match test_value {
            Some((pc, error, added)) => {
                for _ in 0..added {
                    let level = self.level.expect("level");
                    self.actors.push(level);
                }
                (pc, error)
            }
            None => {
                let f = self.find_function(game, "Login", false).ok_or_else(|| VmError::UnknownFunction("Login".into()))?;
                let args = vec![Some(Value::Str(portal.to_string())), Some(Value::Str(options.to_string())), Some(Value::Str(String::new()))];
                let (pc, outs) = self.call(f, game, args)?;
                let error = match outs.get(2) {
                    Some(Some(Value::Str(e))) => e.clone(),
                    _ => String::new(),
                };
                (pc.as_object(), error)
            }
        };
        if !error.is_empty() {
            self.warn(format!("Login : {error}"));
        }
        let spawned = self.actors.len() != actors_before;
        // SpawnPlayActor: without a controller, "Login failed: %s" (NAME_Warning 0x2FF) and nothing else.
        if pc.is_none() {
            self.log.push(format!("Warning: Login failed: {error}"));
        }
        if let Some(pc) = pc {
            self.attach_player(pc, viewport);
            // ULevel::SpawnPlayActor: "<controller> got player <viewport>" after
            // SetPlayer, then Role and RemoteRole, ShowFlags = 0x30CC80C (including SHOW_DistanceFog
            // 0x80000) and RendMap = REN_DynLight.
            if let Some(player) = self.native_prop(pc, Field::Player).as_object() {
                let name = |vm: &Vm, o: ObjId| vm.objects[o].as_ref().map(|x| x.name.clone()).unwrap_or_default();
                self.log.push(format!("Log: {} got player {}", name(self, pc), name(self, player)));
            }
            // Role = ROLE_Authority, RemoteRole = the role given by (push 2:
            // ROLE_SimulatedProxy); the class would have ROLE_DumbProxy.
            self.set_prop(pc, "Role", Value::Byte(4));
            self.set_prop(pc, "RemoteRole", Value::Byte(remote_role));
            self.set_prop(pc, "ShowFlags", Value::Int(0x30cc80c));
            self.set_prop(pc, "RendMap", Value::Int(5));
            // The inventory (travel or default) is accepted before PostLogin; in XIII, this is
            // where the player receives their fists.
            // The inventory: the TravelInfo text under the name from the NAME= option (otherwise, in single player, the first entry), only if Login created
            // actors.
            let travel = if spawned { self.chooses_travel(options, info) } else { None };
            self.accept_travel(game, pc, travel.as_deref());
            // GameInfo::eventPostLogin (ProcessEvent).
            self.event_pub(game, "PostLogin", vec![Value::Object(Some(pc))]);
            // Mods build only: start weapons and ammo of the map (mod_placement.rs).
            #[cfg(feature = "mods")]
            self.apply_mod_start(pc);
        }
        Ok(pc)
    }

    /// Choice of the travel text: FURL::GetOption("NAME=", "") then TMap::Find in the TravelInfo (FString ==: case-insensitive);
    /// with no result and without networking (the LevelInfo's NetMode 0), the first entry.
    fn chooses_travel(&self, options: &str, info: &[(String, String)]) -> Option<String> {
        let name_value = options.split('?').find(|o| o.len() >= 5 && o.is_char_boundary(5) && o[..5].eq_ignore_ascii_case("NAME=")).map_or("", |o| &o[5..]);
        if let Some((_, t)) = info.iter().find(|(k, _)| k.eq_ignore_ascii_case(name_value)) {
            return Some(t.clone());
        }
        let standalone = self.level.is_some_and(|l| self.propr(l, "NetMode").as_byte() == 0);
        info.first().filter(|_| standalone).map(|(_, t)| t.clone())
    }

    /// ULevel::SpawnActor of an actor (bNoCollisionFail zero, like execSpawn):
    /// position, rotation, owner, Tag, Instigator, physics volume and default zone,
    /// added to the octree, PreBeginPlay, BeginPlay, SetZone(0, 1), CheckEncroachment (the actor is
    /// destroyed if it overlaps another), PostBeginPlay, PostNetBeginPlay, SetInitialState, then
    /// FindBase of the placed decorations, pickups and pawns. None if the actor is destroyed.
    #[allow(clippy::too_many_arguments)]
    /// AActor::Spawned (virtual, + 0x8C) of an AMover: BasePos = Location and BaseRot = Rotation.
    pub(crate) fn mover_spawned(&mut self, id: ObjId) {
        if self.is_a(id, "Mover") {
            let (l, r) = (self.native_value(id, Field::Location), self.native_value(id, Field::Rotation));
            self.set_prop(id, "BasePos", l);
            self.set_prop(id, "BaseRot", r);
        }
    }

    pub(crate) fn spawn_actor(&mut self, class: crate::value::ClassId, owner: Value, tag: Option<Value>, location: Value, rotation: Value, instigator: Value, no_collision_fail: bool) -> Result<Option<ObjId>> {
        // ULevel::SpawnActor: refusal of an abstract class
        // (ClassFlags & CLASS_Abstract), of a class that is not an actor, or whose default
        // actor is bStatic or bNoDelete (outside the editor), with a warning.
        let class_name = self.classes[class].name.clone();
        let refusal = if self.classes[class].flags & 1 != 0 {
            Some(format!("class {class_name} is abstract"))
        } else if self.class("Actor").is_none_or(|a| !self.is_child_of(class, a)) {
            Some(format!("{class_name} is not an actor class"))
        } else {
            let cdo = self.classes[class].cdo;
            let no_delete = self.propr(cdo, "bNoDelete").as_bool();
            (self.native_prop(cdo, Field::Static).as_bool() || no_delete).then(|| format!("class {class_name} has bStatic or bNoDelete"))
        };
        if let Some(why) = refusal {
            self.log.push(format!("Warning: SpawnActor failed because {why}"));
            return Ok(None);
        }
        // FindSpot with the default actor's cylinder if bCollideWorld (or bCollideWhenPlacing outside a client),
        // without bNoCollisionFail: the position may be moved; without room, nothing
        // is created.
        let cdo = self.classes[class].cdo;
        let mut location = location;
        let client = self.level.is_some_and(|l| self.propr(l, "NetMode").as_byte() == 3);
        if (self.propr(cdo, "bCollideWorld").as_bool() || (self.propr(cdo, "bCollideWhenPlacing").as_bool() && !client)) && !no_collision_fail {
            let (r, h) = (self.native_prop(cdo, Field::CollisionRadius).as_float(), self.native_prop(cdo, Field::CollisionHeight).as_float());
            let mut l = location.as_vector();
            if !self.find_spot([r, r, h], &mut l) {
                return Ok(None);
            }
            location = Value::vector(l[0], l[1], l[2]);
        }
        let id = self.spawn(class);
        // bRemoteOwned (UActorChannel::ReceivedBunch): Role and RemoteRole swapped.
        if self.ipdrv.spawn_remote_owned {
            let (r, rr) = (self.native_prop(id, Field::Role).clone(), self.native_prop(id, Field::RemoteRole).clone());
            self.set_prop(id, "Role", rr);
            self.set_prop(id, "RemoteRole", r);
        }
        // Tag = class name (execSpawn then replaces it with SpawnTag).
        let class_name = self.classes[class].name.clone();
        self.set_prop(id, "Tag", Value::Name(crate::value::NameStruct::from(class_name)));
        if let Some(level) = self.level {
            self.set_prop(id, "Level", Value::Object(Some(level)));
        }
        // bTicked = !XLevel->Ticked (0x10388B6x): not handled yet in this frame.
        self.set_prop(id, "bTicked", Value::Bool(!self.level_ticked));
        // A spawned actor has no brush (Brush = None, 0x10388C5x).
        if !matches!(self.prop(id, "Brush"), Value::Object(None) | Value::Void) {
            self.set_prop(id, "Brush", Value::Object(None));
        }
        self.set_prop(id, "Location", location);
        self.set_prop(id, "Rotation", rotation);
        self.actors.push(id);
        self.actors_changed();
        // FCollisionHash::AddActor if bCollideActors (before the zone and the physics volume).
        self.refresh_collider(id);
        // Region: LevelInfo, iLeaf -1, ZoneNumber 0; the LevelInfo's PhysicsVolume, then SetZone(0, 1) after
        // BeginPlay (zone, physics volume and, for a pawn, head volume).
        if let Some(level) = self.level {
            if let Value::Struct(n, mut f) = self.native_value(id, Field::Region) {
                f[0] = Value::Object(Some(level));
                f[1] = Value::Int(-1);
                f[2] = Value::Byte(0);
                self.set_prop(id, "Region", Value::Struct(n, f));
            }
        }
        if let Some(volume) = self.default_volume {
            self.set_prop(id, "PhysicsVolume", Value::Object(Some(volume)));
        }
        let skip_bytes = false;
        if !skip_bytes {
            self.set_owner(id, owner.as_object());
        }
        if let Some(tag) = tag {
            self.set_prop(id, "Tag", tag);
        }
        self.set_prop(id, "Instigator", instigator);
        let skip_bytes = false;
        if !skip_bytes {
            self.init_execution(id);
        }
        // During ULevel::Tick, the main loop handles it at its place in Actors; NewlySpawned only receives it at the end
        // of SpawnActor.
        if self.in_tick {
            self.spawn_order.push(id);
        }
        let skip_bytes = false;
        if !skip_bytes {
            self.mover_spawned(id);
        }
        // Mods build only: values of an actor a mod adds, before its PreBeginPlay.
        #[cfg(feature = "mods")]
        self.apply_mod_spawn_values(id);
        // Co-op extra enemies (coop_enemies.rs): a copy takes its original's properties.
        if self.coop.enemies.template.is_some() {
            self.coop_enemies_spawn_values(id);
        }
        for event in ["PreBeginPlay", "BeginPlay"] {
            if let Some(f) = self.find_function(id, event, false) {
                self.call(f, id, vec![])?;
            }
        }
        if self.objects[id].is_none() || self.native_prop(id, Field::DeleteMe).as_bool() {
            return Ok(None);
        }
        self.set_zone(id, false, true);
        self.update_leaves(id);
        let (l, r) = (self.native_prop(id, Field::Location).as_vector(), self.native_prop(id, Field::Rotation).as_rotator());
        // Without bNoCollisionFail: CheckEncroachment, then FCollisionHash::MoveActor
        // (+ 8) if bCollideActors.
        if !no_collision_fail {
            if self.check_encroachment(id, l, r, true) {
                let skip_bytes = false;
                if !skip_bytes {
                    let _ = crate::natives::call(self, "Destroy", id, &mut [], &[]);
                }
                return Ok(None);
            }
            if self.native_prop(id, Field::CollideActors).as_bool() {
                self.octree_move(id);
            }
        }
        // PostNetBeginPlay except on a client (NetMode 3): it comes with the
        // first received properties.
        let client = self.level.is_some_and(|l| self.propr(l, "NetMode").as_byte() == 3);
        for event in ["PostBeginPlay", "PostNetBeginPlay", "SetInitialState"] {
            if client && event == "PostNetBeginPlay" {
                continue;
            }
            if let Some(f) = self.find_function(id, event, false) {
                self.call(f, id, vec![])?;
            }
        }
        if self.objects[id].is_some()
            && self.native_prop(id, Field::Base).as_object().is_none()
            && self.propr(id, "bCollideWorld").as_bool()
            && (self.is_a(id, "Decoration") || self.is_a(id, "Pickup") || self.is_a(id, "Pawn"))
            && matches!(self.native_prop(id, Field::Physics).as_byte(), 0 | 5)
        {
            self.find_base(id);
        }
        // NewlySpawned: at the head, after FindBase, during ULevel::Tick only.
        if self.in_tick && self.objects[id].is_some() {
            self.newly_spawned.push(id);
        }
        // ULevel::SpawnActor: AddDynLight if LightType and neither bStatic nor bNoDelete.
        if self.objects[id].is_some() && self.native_prop(id, Field::LightType).as_byte() != 0 && !self.native_prop(id, Field::Static).as_bool() && !self.pb(id, "bNoDelete") {
            self.add_dyn_light(id);
        }
        // End of ULevel::SpawnActor: RefreshDynDisp if the actor is visible.
        if self.objects[id].is_some() && !self.native_prop(id, Field::Hidden).as_bool() && self.native_prop(id, Field::DrawType).as_byte() != 0 {
            self.refresh_dyn_disp(id);
        }
        Ok(Some(id))
    }

    /// Start of a map in the order of UGameEngine::LoadMap: default physics
    /// volume and InitExecution of each actor, bBegunPlay and bStartup, InitGame, then
    /// PreBeginPlay, BeginPlay, SetZone(1, 1) of each actor, SetVolumes of the pawns,
    /// PostBeginPlay, PostNetBeginPlay, SetInitialState.
    pub fn begin_play(&mut self) {
        let _math = self.session.math.enter();
        // Mods build only: mod values written on the actors the map placed, before any script
        // of the level runs (mod_defaults.rs).
        #[cfg(feature = "mods")]
        {
            // The checkpoint the level starts from, while all its save points are there.
            self.mods.checkpoint = None;
            self.mods.checkpoint = Some(self.mod_checkpoint_here());
            self.apply_mod_placed();
            self.apply_mod_changes();
            self.report_map_mod_only();
            self.apply_mod_removals();
        }
        // ULevel::UpdateTerrainArrays: the map's terrains for SingleLineCheck / MultiLineCheck.
        self.prepare_terrain_collision();
        // The map's actors are in the collision hash from load time, before
        // BeginPlay: the player's pawn (SpawnActor then CheckEncroachment) touches what it
        // overlaps from its creation, before the first ULevel::Tick (Base01's XIIISaveGameTrigger
        // under the PlayerStart: save before MapInfo.FirstFrame, as in the original XIII.log).
        self.refresh_colliders();
        // ULevel::SetActorCollision(1): collision hash, before bBegunPlay.
        self.octree_build();
        // Like the engine: Level.bStartup is true while the map's actors start
        // (this is what makes pawns placed in the editor create their controller).
        if let Some(level) = self.level {
            // UGameEngine::LoadMap: new map (bBegunPlay zero), ULevel::TimeSeconds
            // (+0xD0) = 0; ULevel::Serialize reads the saved time without keeping it.
            // Then GetLevelInfo()->TimeSeconds (+0x314) = 0: scripts and SpawnActor during
            // the load (CanSplash of the pawn in a WaterVolume of SPADS02a) see 0, not the saved
            // time. Saved game (bBegunPlay): the LevelInfo's TimeSeconds.
            // New map (UGameEngine::LoadMap): "Level is %s", "*** WARNING -
            // PATHS MAY NOT BE VALID ***" without bPathsRebuilt (bit 0x80 of +0x380), "Bringing %s
            // up for play (%i)..." with appRound(GetMaxTickRate()) (120 without a NetDriver).
            if !self.native_prop(level, Field::BegunPlay).as_bool() {
                let full = format!("Level {}.MyLevel", self.map_package.map(|p| self.packages[p].name.clone()).unwrap_or_default());
                self.log.push(format!("Log: Level is {full}"));
                if !self.propr(level, "bPathsRebuilt").as_bool() {
                    self.log.push("Log: *** WARNING - PATHS MAY NOT BE VALID ***".into());
                }
                let server = self.ipdrv.net_driver.as_ref().is_some_and(|d| d.server_conn.is_none());
                let rate = if server { self.net_max_tick_rate() } else { 120.0 };
                self.log.push(format!("Log: Bringing {full} up for play ({})...", rate.round_ties_even() as i32));
            }
            self.level_time = if self.native_prop(level, Field::BegunPlay).as_bool() {
                self.native_prop(level, Field::TimeSeconds).as_float() as f64
            } else {
                self.set_prop(level, "TimeSeconds", Value::Float(0.0));
                0.0
            };
        }
        // Default physics volume (ALevelInfo::GetDefaultPhysicsVolume): the LevelInfo's,
        // otherwise a DefaultPhysicsVolume. UGameEngine::LoadMap (0x1037C9xx, before InitGame), for
        // each actor: LastRenderTime (+0x28) = 0, Touching (+0xB4) emptied, PhysicsVolume (+0xC8)
        // = the default volume, whatever the one saved with the map (SetZone redoes it
        // after BeginPlay).
        {
            if let Some(volume) = self.default_volume.or_else(|| self.default_physics_volume()) {
                for a in self.actors.clone() {
                    if self.prop_slot(a, "LastRenderTime").is_ok() {
                        self.set_prop(a, "LastRenderTime", Value::Float(0.0));
                    }
                    self.set_touching(a, Vec::new());
                    if self.prop_slot(a, "PhysicsVolume").is_ok() {
                        self.set_prop(a, "PhysicsVolume", Value::Object(Some(volume)));
                    }
                    if self.propr(a, "HeadVolume").as_object().is_none() && self.prop_slot(a, "HeadVolume").is_ok() {
                        self.set_prop(a, "HeadVolume", Value::Object(Some(volume)));
                    }
                }
            }
        }
        // UGameEngine::LoadMap: InitExecution (vt 0x44) of each actor of the map, after their
        // physics volume.
        for a in self.actors.clone() {
            self.init_execution(a);
        }
        if let Err(e) = self.load_map_init_game() {
            self.warn(format!("InitGame: {e}"));
        }
        let actors = self.actors.clone();
        // UGameEngine::LoadMap: loops over ULevel::Actors, whose entries for
        // actors destroyed in the meantime are null (Destroy during PostBeginPlay: no
        // SetInitialState, hence no BeginState of the automatic state).
        let live = |vm: &Vm, a: ObjId| vm.objects[a].is_some() && !vm.native_prop(a, Field::DeleteMe).as_bool();
        for event in ["PreBeginPlay", "BeginPlay"] {
            for &a in &actors {
                if live(self, a) {
                    self.event(a, event, vec![]);
                }
            }
        }
        for &a in &actors {
            self.set_zone(a, true, true);
        }
        for &a in &actors {
            if self.objects[a].is_some() && self.is_a(a, "Pawn") {
                self.set_volumes(a);
            }
        }
        for event in ["PostBeginPlay", "PostNetBeginPlay", "SetInitialState"] {
            for &a in &actors {
                if live(self, a) {
                    self.event(a, event, vec![]);
                    // UGameEngine::LoadMap: after the event, AActor::PostBeginPlay
                    // (vt 0xF4) of the actor if it is still in ULevel::Actors.
                    if event == "PostBeginPlay" && self.actors.contains(&a) {
                        self.native_post_begin_play(a);
                    }
                }
            }
        }
        // UGameEngine::LoadMap, before bStartup = 0: for each actor
        // of ULevel::Actors, with an AttachTag, SetBase (vt 0xBC) on the first actor whose Tag
        // (+0x44) equals this AttachTag (+0xFC), floor (0, 0, 1), without notification (Banque01: the
        // briefcase and the Porte20 drawer that carries it away); without an AttachTag, FindBase of
        // decorations, pickups and pawns without a Base, bCollideWorld, Physics PHYS_None or PHYS_Rotating.
        for &a in &actors {
            if self.objects[a].is_none() {
                continue;
            }
            let attach = self.prop(a, "AttachTag").as_name();
            if !attach.is_empty() && !attach.eq_ignore_ascii_case("None") {
                let base = self.actors.iter().copied().find(|&b| self.objects[b].is_some() && self.prop(b, "Tag").as_name().eq_ignore_ascii_case(&attach));
                if let Some(b) = base {
                    self.set_base_full(a, Some(b), [0.0, 0.0, 1.0], false);
                }
            } else if self.native_object(a, Field::Base).is_none()
                && self.propr(a, "bCollideWorld").as_bool()
                && matches!(self.native_prop(a, Field::Physics).as_byte(), 0 | 5)
                && (self.is_a(a, "Decoration") || self.is_a(a, "Pickup") || self.is_a(a, "Pawn"))
            {
                self.find_base(a);
            }
        }
        // Mods build only: actors added by mods, spawned while Level.bStartup is still true so a
        // character gets its controller as the placed ones do (mod_placement.rs).
        #[cfg(feature = "mods")]
        self.apply_mod_additions();
        // Mods build only: placed characters a `change` gives a team or can_die (their AI
        // controllers exist now, mod_change.rs).
        #[cfg(feature = "mods")]
        self.take_mod_changed_characters();
        if let Some(level) = self.level {
            self.set_prop(level, "bStartup", Value::Bool(false));
        }
        self.sort_actor_list();
    }

    /// UGameEngine::LoadMap after BeginPlay (0x1037CFxx..): ULevel::Actors rebuilt with
    /// Actors(0) and Actors(1), then the bStatic actors without bAlwaysRelevant (iFirstNetRelevantActor
    /// = their end), the bStatic bAlwaysRelevant ones (iFirstDynamicActor = their end), then the others;
    /// null entries disappear.
    fn sort_actor_list(&mut self) {
        let list: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some())).collect();
        let head = self.level_head.min(list.len());
        let mut sorted: Vec<ObjId> = list.iter().take(head).copied().collect();
        let rest = list.get(head..).unwrap_or(&[]);
        let is_static = |vm: &Vm, a: ObjId| vm.native_prop(a, Field::Static).as_bool();
        let relevant = |vm: &Vm, a: ObjId| vm.propr(a, "bAlwaysRelevant").as_bool();
        sorted.extend(rest.iter().copied().filter(|&a| is_static(self, a) && !relevant(self, a)));
        sorted.extend(rest.iter().copied().filter(|&a| is_static(self, a) && relevant(self, a)));
        self.first_dynamic = sorted.len();
        sorted.extend(rest.iter().copied().filter(|&a| !is_static(self, a)));
        self.actors = sorted;
        self.actors_changed();
    }
}

// ---- Physics (walking, falling, projectiles) ----

/// "Probe" events, by bit number of ProbeMask/IgnoreMask: the 64 fixed names of Core.dll
/// with FName indices 300 to 363 (IsProbing, ProcessEvent: bit = index − 300), read
/// from the original's name table; confirmed by the masks of the game's 1,976 classes and states
/// (`oxiii probes`). A state can ignore these events
/// (`ignores`), and Disable/Enable turn them off or back on until the next GotoState.
pub const PROBES: [&str; 64] = [
    "Spawned",
    "Destroyed",
    "GainedChild",
    "LostChild",
    "Probe4",
    "Probe5",
    "Trigger",
    "UnTrigger",
    "Timer",
    "HitWall",
    "Falling",
    "Landed",
    "PhysicsVolumeChange",
    "Touch",
    "UnTouch",
    "Bump",
    "BeginState",
    "EndState",
    "BaseChange",
    "Attach",
    "Detach",
    "ActorEntered",
    "ActorLeaving",
    "KillCredit",
    "AnimEnd",
    "EndedRotation",
    "InterpolateEnd",
    "EncroachingOn",
    "EncroachedBy",
    "NotifyTurningInPlace",
    "HeadVolumeChange",
    "PainTimer",
    "SpeechTimer",
    "MayFall",
    "CheckDirectionChange",
    "Die",
    "Tick",
    "PlayerTick",
    "Expired",
    "CheckMovementTransition",
    "SeePlayer",
    "EnemyNotVisible",
    "HearNoise",
    "UpdateEyeHeight",
    "SeeMonster",
    "SeeFriend",
    "SpecialHandling",
    "BotDesireability",
    "NotifyBump",
    "NotifyPhysicsVolumeChange",
    "AIHearSound",
    "NotifyHeadVolumeChange",
    "NotifyLanded",
    "NotifyHitWall",
    "Probe54",
    "Probe55",
    "Probe56",
    "Probe57",
    "Probe58",
    "Probe59",
    "Probe60",
    "Probe61",
    "Probe62",
    "EndOfVoice",
];

pub fn probe_bit(name: &str) -> Option<u32> {
    // Cache by address of the name (callers mostly pass literals: "Tick", "Timer"…): 64 slots
    // per thread, validated by the length and exact bytes of the name (a freed string may be
    // replaced by another at the same address).
    thread_local! {
        static CACHE: std::cell::RefCell<[(usize, u8, [u8; 32], Option<u32>); 64]> = const { std::cell::RefCell::new([(0, 0, [0; 32], None); 64]) };
    }
    let active_value = true;
    if !active_value || name.len() > 32 || name.is_empty() {
        return probe_bit_compute(name);
    }
    let p = name.as_ptr() as usize;
    let i = ((p >> 3) ^ (p >> 11) ^ name.len()) & 63;
    let found_value = CACHE.with(|c| {
        let c = c.borrow();
        let e = &c[i];
        (e.0 == p && e.1 as usize == name.len() && (&e.2[..name.len()] == name.as_bytes())).then_some(e.3)
    });
    if let Some(r) = found_value {
        return r;
    }
    let r = probe_bit_compute(name);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let mut bytes_value = [0u8; 32];
        bytes_value[..name.len()].copy_from_slice(name.as_bytes());
        c[i] = (p, name.len() as u8, bytes_value, r);
    });
    r
}

/// probe_bit without cache.
pub fn probe_bit_compute(name: &str) -> Option<u32> {
    // Lowercase → bit table (first rank, like position) instead of 64 comparisons per event.
    // Probe names are ASCII: a name that is not equals none of them.
    static TABLE: std::sync::OnceLock<crate::types::HashMap<String, u32>> = std::sync::OnceLock::new();
    if !name.is_ascii() {
        return None;
    }
    let table = TABLE.get_or_init(|| {
        let mut t = crate::types::HashMap::default();
        for (b, p) in PROBES.iter().enumerate() {
            t.entry(p.to_ascii_lowercase()).or_insert(b as u32);
        }
        t
    });
    crate::with_lower(name, |n| table.get(n).copied())
}

/// FCoords::OrthoRotation (same code for the script
/// native): angles truncated by _ftol, 32768 / π (double). FPU at
/// 24-bit precision: fpatan is not subject to it, but the products, sums and FSQRT are;
/// the angle is rounded to single precision before truncation. Roll: atan2(Z · S.Y, Y · S.Y) with
/// S = GMath.UnitCoords / (pitch, yaw, 0), dot products summed (z + y) + x.
pub fn ortho_rotation(axes: [[f32; 3]; 3]) -> [i32; 3] {
    let k = 10430.37806021874f64;
    let angle = |a: f64| ((a * k) as f32) as i32;
    let [x, y, z] = axes;
    let yaw = angle((x[1] as f64).datan2(x[0] as f64));
    let r = crate::physics::r24x;
    let xy = r(r(r(x[1] as f64 * x[1] as f64) + r(x[0] as f64 * x[0] as f64)).sqrt());
    let pitch = angle((x[2] as f64).datan2(xy));
    // Y axis of the frame without roll.
    let s = crate::ai_natives::axes([pitch, yaw, 0])[1];
    let dot = |a: [f32; 3]| (s[2] * a[2] + s[1] * a[1]) + s[0] * a[0];
    let roll = angle((dot(z) as f64).datan2(dot(y) as f64));
    [pitch, yaw, roll]
}

impl Vm {
    /// Registers a movable object (StaticMesh Mover); `local` are its drawn triangles (mesh
    /// frame). Its collision shape is that of its StaticMesh (`static_collision`).
    pub fn set_mover_shape(&mut self, obj: ObjId, local: Vec<[[f32; 3]; 3]>) {
        self.movers.insert(obj, MoverShape { local });
        self.update_mover_shape(obj);
    }

    /// Recomputes the shape of a movable object at its current position and rotation.
    pub(crate) fn update_mover_shape(&mut self, obj: ObjId) {
        self.sm_shape_update(obj);
    }

    /// True if `a` belongs to `b` (Owner chain) or if it is the same actor.
    pub fn owned_by(&self, a: ObjId, b: ObjId) -> bool {
        let mut cur = Some(a);
        for _ in 0..8 {
            match cur {
                Some(c) if c == b => return true,
                Some(c) => cur = self.native_prop(c, Field::Owner).as_object(),
                None => return false,
            }
        }
        false
    }

    /// ULevel::SingleLineCheck(Hit, source, end, start, TRACE_World, extent) for the natives that
    /// trace the map and the Movers (APositionInfo::execAutoPosition of XIDCine.dll:
    /// TRACE_World, without extent). `actors` adds the pawns and the other actors. Returns
    /// (Hit.Time, Hit.Normal, Hit.Actor) if there is a contact.
    pub fn line_check(&self, source: ObjId, start: [f32; 3], end: [f32; 3], extent: [f32; 3], actors: bool) -> Option<(f32, [f32; 3], Option<ObjId>)> {
        let mut hit = crate::physics::CheckResult::new(1.0);
        let flags = crate::physics::TRACE_WORLD | if actors { crate::physics::TRACE_PAWNS | crate::physics::TRACE_OTHERS } else { 0 };
        self.single_line_check(&mut hit, Some(source), end, start, flags, extent);
        hit.actor.map(|o| (hit.time, hit.normal, Some(o)))
    }

    /// At the start of the tick: brush boxes, and shapes of the octree's StaticMeshes whose
    /// mesh has changed (the script assigns StaticMesh without going through the hash, whereas
    /// GetPrimitive reads it on every trace).
    pub fn refresh_colliders(&mut self) {
        // List in a kept buffer (no allocation or sort per frame, octree.rs).
        thread_local! {
            static BUFFER: std::cell::RefCell<Vec<ObjId>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        let mut list_value = BUFFER.with(|t| std::mem::take(&mut *t.borrow_mut()));
        self.octree_actors_in(&mut list_value);
        // Fast path (the old path runs sm_shape_update for each one): an actor
        // whose Props generation is that of its last sm_shape_update here is skipped without a
        // call or lookup (sm_shape_update would have skipped it: nothing can have changed).
        let fast = !crate::physics::coll::prior(crate::physics::coll::SHAPES)
            && !crate::physics::coll::prior(crate::physics::coll::LISTS)
           ;
        if fast {
            let mut people = self.octree_people_take(list_value.last().map_or(0, |&a| a + 1));
            for &a in &list_value {
                let k = a;
                let Some(g) = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.props.generation()) else { continue };
                if people[k] == g {
                    continue;
                }
                self.sm_shape_update(a);
                people[k] = g;
            }
            self.octree_people_render(people);
        } else {
            for &a in &list_value {
                if self.alive(a) {
                    self.sm_shape_update(a);
                }
            }
        }
        BUFFER.with(|t| *t.borrow_mut() = list_value);
        self.refresh_brush_boxes();
    }

    /// Actors in contact with `a`: the Actor.Touching array (+0xB4), in contact order,
    /// the one the scripts read (TouchTrigger.Trigger, DamageVolume, XIIIPlayerPawn).
    pub fn touching_of(&self, a: ObjId) -> Vec<ObjId> {
        match self.propr(a, "Touching") {
            Value::Array(items) => items.iter().filter_map(|v| v.as_object()).collect(),
            _ => Vec::new(),
        }
    }

    /// Replaces the Actor.Touching array of `a`.
    pub(crate) fn set_touching(&mut self, a: ObjId, list: Vec<ObjId>) {
        if self.prop_slot(a, "Touching").is_ok() {
            self.set_prop(a, "Touching", Value::Array(list.into_iter().map(|o| Value::Object(Some(o))).collect()));
        }
    }

    /// AActor::SetBase: new base; the old one receives Detach and the new one
    /// Attach (except the world geometry), the relative position is noted, then BaseChange.
    pub(crate) fn set_base(&mut self, a: ObjId, new_base: Option<ObjId>, notify: bool) {
        let mut cur = new_base;
        for _ in 0..64 {
            match cur {
                Some(b) if b == a => return,
                Some(b) => cur = self.native_prop(b, Field::Base).as_object(),
                None => break,
            }
        }
        let old = self.native_prop(a, Field::Base).as_object();
        if new_base != old {
            if let Some(o) = old.filter(|&o| self.objects[o].is_some()) {
                if !self.propr(o, "bWorldGeometry").as_bool() {
                    // TArray::RemoveItem: all occurrences.
                    if let Value::Array(mut items) = self.prop(o, "Attached") {
                        let n = items.len();
                        items.retain(|x| x.as_object() != Some(a));
                        if items.len() != n {
                            self.set_prop(o, "Attached", Value::Array(items));
                        }
                    }
                    self.event(o, "Detach", vec![Value::Object(Some(a))]);
                }
            }
            self.set_prop(a, "Base", Value::Object(new_base));
            if let Some(b) = new_base {
                if self.propr(a, "AttachmentBone").as_name().is_empty() {
                    let (l, bl) = (self.native_prop(a, Field::Location).as_vector(), self.native_prop(b, Field::Location).as_vector());
                    self.set_prop(a, "RelativeLocation", Value::vector(l[0] - bl[0], l[1] - bl[1], l[2] - bl[2]));
                    self.update_relative_rotation(a);
                }
            }
            if let Some(b) = self.native_prop(a, Field::Base).as_object() {
                if !self.propr(b, "bWorldGeometry").as_bool() {
                    if let Value::Array(mut items) = self.prop(b, "Attached") {
                        items.push(Value::Object(Some(a)));
                        self.set_prop(b, "Attached", Value::Array(items));
                    }
                    self.event(b, "Attach", vec![Value::Object(Some(a))]);
                }
            }
            if notify {
                self.event(a, "BaseChange", vec![]);
            }
        }
        match self.native_prop(a, Field::Base).as_object() {
            Some(b) if !self.propr(b, "bWorldGeometry").as_bool() => {}
            _ => self.set_prop(a, "AttachmentBone", Value::Name(crate::value::NameStruct::from(String::new()))),
        }
    }

    /// AActor::UpdateRelativeRotation: rotation expressed in the base's frame,
    /// OrthoRotation((GMath.UnitCoords / Rotation) / OrthoRotation(GMath.UnitCoords * Base.Rotation));
    /// the base's rotation goes through a frame again, hence small remainders (4 units).
    pub(crate) fn update_relative_rotation(&mut self, a: ObjId) {
        use openxiii_assets::coords;
        let Some(b) = self.native_prop(a, Field::Base).as_object() else {
            return;
        };
        if self.propr(b, "bWorldGeometry").as_bool() || !self.propr(a, "AttachmentBone").as_name().is_empty() {
            return;
        }
        let base = coords::mul_rotator(&coords::UNIT, self.native_prop(b, Field::Rotation).as_rotator());
        let base_rot = ortho_rotation([base.x, base.y, base.z]);
        let me = coords::div_rotator(&coords::UNIT, self.native_prop(a, Field::Rotation).as_rotator());
        let local = coords::div_rotator(&me, base_rot);
        let r = ortho_rotation([local.x, local.y, local.z]);
        self.set_prop(a, "RelativeRotation", Value::rotator(r[0], r[1], r[2]));
    }

    /// AController::Tick, end: a pawn turning towards DesiredRotation aims at the Focus
    /// (otherwise the FocalPoint) through rotateToward; the controller takes the pawn's rotation.
    pub(crate) fn controller_rotation(&mut self, c: ObjId) {
        let Some(pawn) = self.native_prop(c, Field::Pawn).as_object().filter(|&p| self.objects[p].is_some()) else {
            return;
        };
        if self.propr(pawn, "bRotateToDesired").as_bool() {
            if let Some(f) = self.propr(c, "Focus").as_object().filter(|&f| self.objects[f].is_some()) {
                let l = self.native_value(f, Field::Location);
                self.set_prop(c, "FocalPoint", l);
            }
            let focal = self.propr(c, "FocalPoint").as_vector();
            self.rotate_toward(pawn, focal);
        }
        let r = self.native_value(pawn, Field::Rotation);
        self.set_prop(c, "Rotation", r);
    }
}

impl Vm {
    /// AActor::PostBeginPlay (vt 0xF4) in C++: empty except AMapInfo::PostBeginPlay (Xiii.dll), which sets MapInfo.TGSDummy (+0x1F8) from Level.iLoadEngineValue (+0x3EC, copied
    /// from iLoadSpecificValue by MapInfo.PreBeginPlay). Some maps' scripts read this value
    /// (PRock01a, USA01…).
    pub(crate) fn native_post_begin_play(&mut self, a: ObjId) {
        if !self.is_a(a, "MapInfo") {
            return;
        }
        let Some(level) = self.native_object(a, Field::Level) else { return };
        let v = match self.propr(level, "iLoadEngineValue").as_int() {
            26 => 546,
            55 => 21627,
            81 => 856,
            106 => 4,
            130 => 69,
            142 => 3589,
            191 => 703,
            _ => return,
        };
        self.set_prop(a, "TGSDummy", Value::Int(v));
    }
}

/// Flags read by AActor::CheckCollisionType, in the order of its table's columns.
pub(crate) const FLAGS_COL_CATEGORY: [&str; 12] = [
    "bWorldGeometry",
    "bCollideActors",
    "bCollideWorld",
    "bBlockActors",
    "bBlockPlayers",
    "bBlockZeroExtentTraces",
    "bBlockNonZeroExtentTraces",
    "bCanSeeThrough",
    "bCanShootThroughWithRayCastingWeapon",
    "bCanShootThroughWithProjectileWeapon",
    "bProjTarget",
    "bUseCylinderCollision",
];

/// AActor::CheckCollisionType (called by AActor::PostLoad): ColCategory
/// (+0x1A2) = index (1 to 11, ColType_NoCol to ColType_PickupArmeImpro) of the first row of the
/// table (filled) equal to the flags, 0 (ColType_User) otherwise.
pub(crate) fn check_collision_type(flag_bits: [bool; 12]) -> u8 {
    const TABLE: [[u8; 12]; 11] = [
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [1, 1, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0],
        [1, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0],
        [1, 1, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0],
        [0, 1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 0],
        [0, 1, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0],
        [0, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0],
        [0, 1, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0],
        [0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 1, 1],
        [0, 1, 1, 0, 0, 1, 1, 1, 1, 1, 0, 1],
        [0, 1, 1, 0, 0, 1, 1, 1, 0, 0, 0, 1],
    ];
    TABLE.iter().position(|l| l.iter().zip(flag_bits).all(|(&t, d)| (t != 0) == d)).map_or(0, |i| i as u8 + 1)
}

impl Vm {
    /// Elements of TraceActors (AActor::execTraceActors): the contacts of ULevel::MultiLineCheck
    /// (TraceType | 0x9F) of the requested class, in order; each element is [actor, HitLoc,
    /// HitNorm] (+ the bone name with TRACETYPE_RequestBones).
    pub(crate) fn trace_actors_items(&mut self, this: ObjId, class: Option<crate::value::ClassId>, end: [f32; 3], start: [f32; 3], extent: [f32; 3], trace_type: u32) -> Vec<Value> {
        let matches_class = |vm: &Vm, o: ObjId| class.is_none_or(|c| vm.objects[o].as_ref().is_some_and(|x| vm.is_child_of(x.class, c)));
        // TRACETYPE_RequestBones: GetLastTraceBone is NAME_None at the start, then the bone of
        // each contact of the class that has one (set on its loop iteration, see set_iteration).
        let bones = trace_type & crate::bone_hit::TRACE_REQUEST_BONES != 0;
        if bones {
            self.last_trace_bone.clear();
        }
        // The loop body runs for each contact in the list, of the requested class or
        // not (without a match, the jump goes straight to the body); without a match,
        // Actor, HitLoc and HitNorm keep their value (Void element).
        let hits = self.multi_line_check(end, start, extent, true, trace_type | 0x9f, Some(this));
        let mut items = Vec::new();
        for mut h in hits {
            if !h.actor.is_some_and(|o| self.objects[o].is_some() && matches_class(self, o)) {
                items.push(Value::Void);
                continue;
            }
            let mut parts = vec![
                Value::Object(h.actor),
                Value::vector(h.location[0], h.location[1], h.location[2]),
                Value::vector(h.normal[0], h.normal[1], h.normal[2]),
            ];
            if bones {
                self.skeletal_bone_check(&mut h, end, start, trace_type | 0x9f);
                if let Some(bone) = self.trace_bone_name(&h) {
                    parts.push(Value::Name(crate::value::NameStruct::from(bone)));
                }
            }
            items.push(Value::Array(parts));
        }
        items
    }
}

/// TravelInfo with a single text, stored under the name from the options' NAME= option (what the callers of `spawn_play_actors` give).
pub(crate) fn travel_info_named(options: &str, travel: Option<&str>) -> Vec<(String, String)> {
    let name_value = options.split('?').find(|o| o.len() >= 5 && o.is_char_boundary(5) && o[..5].eq_ignore_ascii_case("NAME=")).map_or("", |o| &o[5..]);
    travel.map(|t| vec![(name_value.to_string(), t.to_string())]).unwrap_or_default()
}

/// ParseNext: spaces, tabs, line ends and "; …" comments skipped; returns the position of the
/// first useful character (the end of the text stands for the final zero).
pub(crate) fn parse_next(chars: &[char], mut p: usize) -> usize {
    loop {
        while matches!(chars.get(p), Some(' ' | '\t' | '\r' | '\n')) {
            p += 1;
        }
        if chars.get(p) != Some(&';') {
            return p;
        }
        while p < chars.len() && !matches!(chars[p], '\n' | '\r') {
            p += 1;
        }
    }
}

