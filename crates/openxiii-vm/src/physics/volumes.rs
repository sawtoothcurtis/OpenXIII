//! Volumes and zones, reimplemented from Engine.dll: brush (UModel) of each volume of the map,
//! AVolume::Encompasses, ALevelInfo::GetDefaultPhysicsVolume,
//! AActor::SetZone, APawn::SetZone, APhysicsVolume::SetZone
//! and AActor::SetVolumes. The traces and contacts with the brushes
//! (UModel::LineCheck / PointCheck) are in `physics.rs`, the BSP in `openxiii_assets::bsp`.

use crate::NativeField as Field;
use crate::physics::V;
use crate::value::{ObjId, Value};
use crate::Vm;
use openxiii_assets::bsp::{self, BspHit, BspModel, Frame, Scale};
use openxiii_package::ObjectRef;
use std::collections::{HashMap, HashSet};

impl Vm {
    /// Map loading: brush of each volume of the level (Brush property, a Model of the
    /// package) and the map's BSP (the largest Model that is no actor's brush) for the
    /// zones, with the actor of each zone.
    pub(crate) fn load_brushes(&mut self, pi: usize, map_objects: &HashMap<usize, ObjId>, actors: &[ObjId]) {
        let Some(volume) = self.class("Volume") else { return };
        let map_value = self.packages[pi].name.clone();
        self.order_original_for(&map_value);
        let in_level: HashSet<ObjId> = actors.iter().copied().collect();
        let mut found = Vec::new();
        let mut errors = Vec::new();
        let mut polys: Vec<(ObjId, Vec<openxiii_assets::bsp::Poly>)> = Vec::new();
        let level = {
            let lp = &self.packages[pi];
            let mut entries: Vec<(usize, ObjId)> = map_objects.iter().map(|(&e, &o)| (e, o)).collect();
            entries.sort();
            for (export, id) in entries {
                if !in_level.contains(&id) || !self.objects[id].as_ref().is_some_and(|o| self.is_child_of(o.class, volume)) {
                    continue;
                }
                let Ok(o) = lp.package.object_data(&lp.data, export) else { continue };
                let Some(openxiii_package::Value::Object(r)) = o.get(&lp.package, "Brush") else { continue };
                let ObjectRef::Export(e) = ObjectRef::from(*r) else { continue };
                if lp.package.export_class(e) != "Model" {
                    continue;
                }
                match BspModel::read(&lp.package, &lp.data, e) {
                    Ok(mut m) => {
                        if let ObjectRef::Export(pe) = ObjectRef::from(m.polys) {
                            if let Ok(p) = bsp::read_polys(&lp.package, &lp.data, pe) {
                                polys.push((id, p));
                            }
                        }
                        // ABrush::PostLoad of the volume (the polygons have just been read for ULevel::PostLoad)
                        m.abrush_post_load_volume();
                        found.push((id, m));
                    }
                    Err(err) => errors.push(format!("brush {}: {err}", lp.package.object_path(ObjectRef::Export(e)))),
                }
            }
            openxiii_assets::model::level_model(&lp.package, &lp.data).and_then(|l| BspModel::read(&lp.package, &lp.data, l).ok())
        };
        for e in errors {
            self.warn(e);
        }
        // ULevel::PostLoad walks the volumes in GObjObjects index order.
        if !self.order_original.is_empty() {
            polys.sort_by_key(|(id, _)| self.key_order(*id));
        }
        self.map_package = Some(pi);
        // Static lights of the level's model (UModel+0x110 / +0x128 / +0x134): calls FLight::Init
        // on the first `static_lights` every frame (scene_lights.rs).
        let light_model = {
            let lp = &self.packages[pi];
            openxiii_assets::model::level_model(&lp.package, &lp.data)
                .and_then(|l| openxiii_assets::model::Model::read(&lp.package, &lp.data, l).ok().map(|m|(l,m)))
        };
        self.static_light_types = light_model.as_ref().map(|(_,m)| (
            m.lights.iter().take(m.static_lights.max(0) as usize).map(|l| l.light_type).collect(), m.refresh_lights != 0,
        )).unwrap_or_default();
        // Metadata of the real Model reader, not a PostLoad initialization.
        // Explicitly clear the map cache even if the number of slots stays the same.
        if let Some((export,m)) = light_model {
            self.scene_light_model_loaded(Some((pi,export)), m.lights, m.static_lights.max(0) as usize, m.refresh_lights != 0);
        } else {
            self.scene_light_model_loaded(None, Vec::new(), 0, false);
        }
        if let Some(m) = level {
            self.zone_actors = m
                .zone_actors
                .iter()
                .map(|&r| match ObjectRef::from(r) {
                    ObjectRef::Export(e) => map_objects.get(&e).copied(),
                    _ => None,
                })
                .collect();
            // Brushes of the surfaces, and polygons of those of the portals and anti-portals
            // (reads them in Brush->Polys).
            self.surf_actors = m
                .surfs
                .iter()
                .map(|s| match ObjectRef::from(s.actor) {
                    ObjectRef::Export(e) => map_objects.get(&e).copied(),
                    _ => None,
                })
                .collect();
            self.brush_polys.clear();
            let lp = &self.packages[pi];
            let mut wanted: Vec<(ObjId, usize)> = Vec::new();
            for (s, a) in m.surfs.iter().zip(&self.surf_actors) {
                let Some(a) = *a else { continue };
                if s.poly_flags & 0x400_0010 == 0 || wanted.iter().any(|w| w.0 == a) {
                    continue;
                }
                let Some(e) = map_objects.iter().find(|(_, &o)| o == a).map(|(&e, _)| e) else { continue };
                let Ok(o) = lp.package.object_data(&lp.data, e) else { continue };
                let Some(openxiii_package::Value::Object(r)) = o.get(&lp.package, "Brush") else { continue };
                if let ObjectRef::Export(b) = ObjectRef::from(*r) {
                    wanted.push((a, b));
                }
            }
            for (a, b) in wanted {
                let polys = BspModel::read(&lp.package, &lp.data, b).ok().and_then(|bm| match ObjectRef::from(bm.polys) {
                    ObjectRef::Export(pe) => bsp::read_polys(&lp.package, &lp.data, pe).ok(),
                    _ => None,
                });
                if let Some(p) = polys {
                    self.brush_polys.insert(a, p);
                }
            }
            self.projectors.node_polys.clear();
            self.level_bsp = Some(m);
        }
        // Materials of the brushes' surfaces (movers): collision labels, for the HitMat
        // that AActor::execTrace derives from Hit.Material (UModel::LineCheck).
        if let Some(pi) = self.map_package {
            let lp = &self.packages[pi];
            let names: Vec<String> = found
                .iter()
                .flat_map(|(_, m): &(ObjId, BspModel)| m.surfs.iter().map(|s| s.material))
                .filter_map(|r| match ObjectRef::from(r) {
                    ObjectRef::None => None,
                    ObjectRef::Export(_) => Some(format!("{}.{}", lp.name, lp.package.object_path(r.into()))),
                    _ => Some(lp.package.object_path(r.into())),
                })
                .collect();
            for n in names {
                self.material_tag(&n);
            }
        }
        self.brushes.extend(found);
        self.refresh_brush_boxes();
        // ULevel::PostLoad: for each water volume, each polygon of the brush
        // facing up (Normal.Z >= 0.9 in the brush) gives a water plane: box of its
        // vertices in the world (ToWorld), centre and half-widths in X, Y, highest Z.
        if let Some(pv) = self.class("PhysicsVolume") {
            for (id, list) in polys {
                if !self.objects[id].as_ref().is_some_and(|o| self.is_child_of(o.class, pv)) || !self.propr(id, "bWaterVolume").as_bool() {
                    continue;
                }
                let frame = self.brush_frame(id);
                for plan in plans_water(&list, &frame.to_world) {
                    self.water_planes.push((plan, id));
                }
            }
        }
    }

    pub(crate) fn obj_label(&self, o: ObjId) -> String {
        self.objects.get(o).and_then(|x| x.as_ref()).map_or_else(|| format!("#{o}"), |x| x.name.clone())
    }

    fn scale_prop(&self, a: ObjId, name: &str) -> Scale {
        self.scale_of(self.propr(a, name))
    }

    /// FScale of a value (Scale, SheerRate, SheerAxis).
    fn scale_of(&self, v: &Value) -> Scale {
        match v {
            Value::Struct(_, f) if f.len() >= 3 => Scale { scale: f[0].as_vector(), sheer_rate: f[1].as_float(), sheer_axis: f[2].as_byte() },
            _ => Scale::default(),
        }
    }

    /// MainScale and PostScale of a brush: through the native fields (same value), or by name
    /// (old path).
    fn ladders_brush(&self, a: ObjId) -> (Scale, Scale) {
        if crate::physics::coll::prior(crate::physics::coll::BRUSHES) {
            return (self.scale_prop(a, "MainScale"), self.scale_prop(a, "PostScale"));
        }
        (self.scale_of(self.lp(a, Field::MainScale, "MainScale")), self.scale_of(self.lp(a, Field::PostScale, "PostScale")))
    }

    /// `is_a(a, "Brush")`: through the class flag (same answer), or by name (old path).
    fn is_brush_actor(&self, a: ObjId) -> bool {
        if crate::physics::coll::prior(crate::physics::coll::BRUSHES) {
            return self.is_a(a, "Brush");
        }
        self.is(a, crate::flags_classes::D::Brush)
    }

    /// Frames of an actor's brush (ABrush::ToLocal / ToWorld) from Location, Rotation,
    /// PrePivot, MainScale and PostScale.
    /// An actor that is not a Brush (XIII's Movers inherit from Actor) takes AActor::ToLocal
    /// / ToWorld, vtable +0xA0 / +0xA4: without PrePivot or scale.
    pub(crate) fn brush_frame(&self, a: ObjId) -> Frame {
        let (l, r, pp) = (self.native_prop(a, Field::Location).as_vector(), self.native_prop(a, Field::Rotation).as_rotator(), self.native_prop(a, Field::PrePivot).as_vector());
        if !self.is_brush_actor(a) {
            return Frame { to_local: bsp::actor_to_local(l, r), to_world: bsp::actor_to_world(l, r) };
        }
        let (main, post) = self.ladders_brush(a);
        Frame { to_local: bsp::brush_to_local(l, r, pp, &main, &post), to_world: bsp::brush_to_world(l, r, pp, &main, &post) }
    }

    /// Bounding boxes of the brushes in the world (UModel::GetCollisionBoundingBox),
    /// recomputed at the start of the tick like the cylinders' positions.
    pub(crate) fn refresh_brush_boxes(&mut self) {
        // Nothing to redo if no entry of `brush_frame` and no raw box has changed (bit for bit)
        // for the same set of live brushes: the list would be the same.
        let mut keys_list = std::mem::take(&mut self.brush_box_keys_buffer);
        keys_list.clear();
        keys_list.extend(self.brushes.iter().filter(|(&o, _)| self.objects.get(o).is_some_and(|x| x.is_some())).map(|(&o, m)| (o, self.key_box_brush(o, m))));
        keys_list.sort_unstable_by_key(|c| c.0);
        if keys_list == self.brush_box_keys {
            self.brush_box_keys_buffer = keys_list;
            return;
        }
        self.brush_box_keys_buffer = std::mem::replace(&mut self.brush_box_keys, keys_list);
        let mut boxes: Vec<(ObjId, V, V)> = self
            .brushes
            .iter()
            .filter(|(&o, _)| self.objects.get(o).is_some_and(|x| x.is_some()))
            .filter_map(|(&o, m)| m.collision_bounding_box(Some(&self.brush_frame(o))).map(|(lo, hi)| (o, lo, hi)))
            .collect();
        boxes.sort_by_key(|b| b.0);
        self.brush_boxes = boxes;
    }

    /// Inputs of a brush's box in the world: those of `brush_frame` and the raw box.
    fn key_box_brush(&self, o: ObjId, m: &bsp::BspModel) -> [u32; 27] {
        let mut k = [0u32; 27];
        let (l, r, pp) = (self.native_prop(o, Field::Location).as_vector(), self.native_prop(o, Field::Rotation).as_rotator(), self.native_prop(o, Field::PrePivot).as_vector());
        let brush = self.is_brush_actor(o);
        let (main, post) = if brush { self.ladders_brush(o) } else { (Scale::default(), Scale::default()) };
        let raw = &m.raw_bounding_box;
        let f = [l[0], l[1], l[2], pp[0], pp[1], pp[2], main.scale[0], main.scale[1], main.scale[2], main.sheer_rate, post.scale[0], post.scale[1], post.scale[2], post.sheer_rate];
        let b = [raw.min[0], raw.min[1], raw.min[2], raw.max[0], raw.max[1], raw.max[2]];
        for (d, x) in k.iter_mut().zip(f.iter().chain(&b)) {
            *d = x.to_bits();
        }
        k[20] = r[0] as u32;
        k[21] = r[1] as u32;
        k[22] = r[2] as u32;
        k[23] = main.sheer_axis as u32;
        k[24] = post.sheer_axis as u32;
        k[25] = raw.valid as u32;
        k[26] = brush as u32;
        k
    }

    /// AVolume::Encompasses: the point is inside the brush (PointCheck without extent).
    pub(crate) fn encompasses(&self, v: ObjId, p: V) -> bool {
        let Some(m) = self.brushes.get(&v) else { return false };
        let mut hit = BspHit::new(1.0);
        !m.point_check(&mut hit, Some(&self.brush_frame(v)), p, [0.0; 3], 0)
    }

    /// Bounding box of this actor's brush in the world (tools: a volume is aimed at
    /// inside its brush, not at its pivot).
    pub fn brush_box(&self, a: ObjId) -> Option<(V, V)> {
        self.brushes.get(&a).and_then(|m| m.collision_bounding_box(Some(&self.brush_frame(a))))
    }

    /// Volumes that contain this point (tools: simulate's driver avoids the LethalVolumes).
    pub fn volumes_at(&self, p: V) -> Vec<ObjId> {
        self.actors.iter().copied().filter(|&a| self.objects[a].is_some() && self.is_a(a, "Volume") && self.encompasses(a, p)).collect()
    }

    /// ALevelInfo::GetDefaultPhysicsVolume: the LevelInfo's PhysicsVolume, otherwise a
    /// DefaultPhysicsVolume created for it (Priority −1000000, bNoDelete).
    pub(crate) fn default_physics_volume(&mut self) -> Option<ObjId> {
        let level = self.level?;
        if let Some(v) = self.native_object(level, Field::PhysicsVolume) {
            self.default_volume = Some(v);
            return Some(v);
        }
        let class = self.class("DefaultPhysicsVolume")?;
        let v = self.spawn(class);
        self.set_native_prop(v, Field::Level, Value::Object(Some(level)));
        self.set_prop(v, "Priority", Value::Int(-1_000_000));
        self.set_prop(v, "bNoDelete", Value::Bool(true));
        self.set_default_zone(v);
        self.set_native_prop(level, Field::PhysicsVolume, Value::Object(Some(v)));
        self.actors.push(v);
        self.actors_changed();
        self.default_volume = Some(v);
        Some(v)
    }

    /// UModel::PointRegion of the map's BSP: (zone, iLeaf, ZoneNumber), the zone being the
    /// LevelInfo outside the ZoneInfos.
    pub(crate) fn point_region(&self, p: V) -> (Option<ObjId>, i32, u8) {
        let Some(m) = &self.level_bsp else { return (self.level, -1, 0) };
        let (_, leaf, number) = m.point_region(p);
        let zone = if m.nodes.is_empty() { None } else { self.zone_actors.get(number as usize).copied().flatten() };
        (zone.or(self.level), leaf, number)
    }

    fn set_region(&mut self, a: ObjId, zone: Option<ObjId>, leaf: i32, number: u8) {
        // Written in place: the same structure as a modified copy set back, without allocation.
        if let Some(Value::Struct(_, f)) = self.native_prop_mut(a, Field::Region) {
            if f.len() >= 3 {
                f[0] = Value::Object(zone);
                f[1] = Value::Int(leaf);
                f[2] = Value::Byte(number);
            }
        }
    }

    fn priority(&self, v: ObjId) -> i32 {
        self.propr(v, "Priority").as_int()
    }

    /// SetZone (vt 0xEC): AActor::SetZone, APawn::SetZone,
    /// APhysicsVolume::SetZone or ALevelInfo::SetZone depending on the class.
    pub(crate) fn set_zone(&mut self, a: ObjId, test: bool, force_refresh: bool) {
        if !self.objects.get(a).is_some_and(|o| o.is_some()) {
            return;
        }
        let kind_value = if self.is_a(a, "LevelInfo") {
            KindZone::LevelEntry
        } else if self.is_a(a, "PhysicsVolume") {
            KindZone::VolumePhysics
        } else if self.is_a(a, "Pawn") {
            KindZone::Pawn
        } else {
            KindZone::Acteur
        };
        set_zone_core(&mut HostZoneVm { vm: self }, a, kind_value, test, force_refresh);
    }

    /// AActor::SetVolumes, called for the pawns when the map is loaded.
    pub(crate) fn set_volumes(&mut self, a: ObjId) {
        set_volumes_core(&mut HostZoneVm { vm: self }, a);
    }
}

/// Version of SetZone called (vtable +0xEC): the actor's class chooses it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum KindZone {
    Acteur,
    Pawn,
    VolumePhysics,
    LevelEntry,
}

/// Region of an actor: (Zone, iLeaf, ZoneNumber), at +0x98.
pub(crate) type Region = (Option<ObjId>, i32, u8);

/// What SetZone reads and calls outside the actor: the VM, or in the tests the answers recorded on
/// Engine.dll.
pub(crate) trait HostZone {
    fn removes(&self, a: ObjId) -> bool; // bDeleteMe, +0x2E bit 0
    fn level_value(&self, a: ObjId) -> Option<ObjId>; // Level, +0x7C
    fn region(&self, a: ObjId) -> Region;
    fn pose_region(&mut self, a: ObjId, r: Region);
    fn location(&self, a: ObjId) -> V; // +0xCC
    /// UModel::PointRegion of XLevel's model, Level as default zone.
    fn point_region(&mut self, level_value: Option<ObjId>, p: V) -> Region;
    /// ALevelInfo::GetDefaultPhysicsVolume.
    fn volume_default(&mut self, level_value: Option<ObjId>) -> Option<ObjId>;
    fn touching(&self, a: ObjId) -> Vec<Option<ObjId>>; // +0xB4
    fn is_volume_physics(&mut self, v: ObjId) -> bool; // IsA(APhysicsVolume)
    /// AVolume::Encompasses.
    fn bounding(&mut self, v: ObjId, p: V) -> bool;
    fn priority_value(&self, v: ObjId) -> i32; // +0x298
    fn volume(&self, a: ObjId) -> Option<ObjId>; // PhysicsVolume, +0xC8
    fn pose_volume(&mut self, a: ObjId, v: Option<ObjId>);
    fn head_value(&self, a: ObjId) -> Option<ObjId>; // HeadVolume, +0x2B0
    fn pose_head(&mut self, a: ObjId, v: Option<ObjId>);
    fn height_eyes(&self, a: ObjId) -> f32; // BaseEyeHeight, +0x28C
    fn controller(&self, a: ObjId) -> Option<ObjId>; // Controller, +0x1F0
    /// Script event `name_value` sent to `target_value` with an object as parameter.
    fn event_value(&mut self, target_value: ObjId, name_value: &'static str, arg: Option<ObjId>);
}

/// Physics volumes of the body (and of the head for a pawn): among the PhysicsVolumes of Touching,
/// the one with the highest priority (strictly higher than the current best, the default
/// volume at the start) that contains the point (loop of AActor::SetZone).
fn best_volumes(h: &mut impl HostZone, a: ObjId, default_value: Option<ObjId>, p: V, eyes: Option<V>) -> (Option<ObjId>, Option<ObjId>) {
    let (mut body, mut head_value) = (default_value, default_value);
    for v in h.touching(a).into_iter().flatten() {
        if !h.is_volume_physics(v) {
            continue;
        }
        if h.bounding(v, p) && body.map_or(true, |c| h.priority_value(v) > h.priority_value(c)) {
            body = Some(v);
        }
        // APawn::SetZone: the head is tested after the body, whether it is inside or not.
        if let Some(y) = eyes {
            if h.bounding(v, y) && head_value.map_or(true, |t| h.priority_value(v) > h.priority_value(t)) {
                head_value = Some(v);
            }
        }
    }
    (body, head_value)
}

/// SetZone(bTest, bForceRefresh). Common part (AActor, APawn,
/// APhysicsVolume): BSP zone at the centre; if it changes, ActorLeaving of the old one
/// and ZoneChange, new Region then ActorEntered (no event in test mode). Then:
///  - actor: outside test mode, the best volume if it changes (ActorLeavingVolume of the old one and
///    PhysicsVolumeChange, then ActorEnteredVolume);
///  - pawn: volume of the body and of the head (BaseEyeHeight + Location.Z), the controller notified
///    (NotifyPhysicsVolumeChange); in test mode the volumes change without events;
///  - physics volume: PhysicsVolume = itself, even in test mode.
/// ALevelInfo::SetZone only does Region = (Level, −1, 0).
pub(crate) fn set_zone_core(h: &mut impl HostZone, a: ObjId, kind_value: KindZone, test: bool, force_refresh: bool) {
    if h.removes(a) {
        return;
    }
    let level_value = h.level_value(a);
    if kind_value == KindZone::LevelEntry {
        h.pose_region(a, (level_value, -1, 0));
        return;
    }
    if force_refresh {
        h.pose_region(a, (level_value, -1, 0));
    }
    let location = h.location(a);
    let new_value = h.point_region(level_value, location);
    let previous_one = h.region(a).0;
    if new_value.0 != previous_one {
        if !test {
            if let Some(z) = previous_one {
                h.event_value(z, "ActorLeaving", Some(a));
            }
            h.event_value(a, "ZoneChange", new_value.0);
        }
        h.pose_region(a, new_value);
        if !test {
            if let Some(z) = new_value.0 {
                h.event_value(z, "ActorEntered", Some(a));
            }
        }
    } else {
        h.pose_region(a, new_value);
    }
    match kind_value {
        KindZone::VolumePhysics => h.pose_volume(a, Some(a)),
        KindZone::Acteur => {
            let default_value = h.volume_default(level_value);
            let (volume, _) = best_volumes(h, a, default_value, location, None);
            let prior = h.volume(a);
            if test || volume == prior {
                return;
            }
            if let Some(o) = prior {
                h.event_value(o, "ActorLeavingVolume", Some(a));
                h.event_value(a, "PhysicsVolumeChange", volume);
            }
            h.pose_volume(a, volume);
            if let Some(v) = volume {
                h.event_value(v, "ActorEnteredVolume", Some(a));
            }
        }
        KindZone::Pawn => {
            let default_value = h.volume_default(level_value);
            let eyes = [location[0], location[1], h.height_eyes(a) + location[2]];
            let (volume, head_value) = best_volumes(h, a, default_value, location, Some(eyes));
            let prior = h.volume(a);
            if volume != prior {
                if !test {
                    if let Some(o) = prior {
                        h.event_value(o, "PawnLeavingVolume", Some(a));
                        h.event_value(a, "PhysicsVolumeChange", volume);
                    }
                    if let Some(c) = h.controller(a) {
                        h.event_value(c, "NotifyPhysicsVolumeChange", volume);
                    }
                }
                h.pose_volume(a, volume);
                if !test {
                    if let Some(v) = volume {
                        h.event_value(v, "PawnEnteredVolume", Some(a));
                    }
                }
            }
            if head_value != h.head_value(a) {
                if !test {
                    h.event_value(a, "HeadVolumeChange", head_value);
                }
                h.pose_head(a, head_value);
            }
        }
        KindZone::LevelEntry => {}
    }
}

/// What SetVolumes reads in addition to SetZone: the level's actors and the Touching lists.
pub(crate) trait HostVolumes: HostZone {
    fn actors_list(&self) -> Vec<Option<ObjId>>; // XLevel->Actors, +0x2C of XLevel
    fn is_volume(&mut self, v: ObjId) -> bool; // IsA(AVolume)
    fn collision_actors(&self, a: ObjId) -> bool; // bCollideActors, +0x34 bit 0x20
    /// Touching.AddItem (FArray::Add, without a duplicate test).
    fn adds_contact(&mut self, a: ObjId, other_one: ObjId);
}

/// AActor::SetVolumes: each volume of the level that contains the actor's centre
/// enters both Touching lists (the volume's first, without Touch) if the actor has
/// bCollideActors; a PhysicsVolume with a strictly higher priority than the current physics
/// volume becomes the physics volume.
pub(crate) fn set_volumes_core(h: &mut impl HostVolumes, a: ObjId) {
    for v in h.actors_list().into_iter().flatten() {
        if !h.is_volume(v) || !h.bounding(v, h.location(a)) {
            continue;
        }
        if h.collision_actors(a) {
            h.adds_contact(v, a);
            h.adds_contact(a, v);
        }
        if h.is_volume_physics(v) && h.volume(a).map_or(true, |c| h.priority_value(v) > h.priority_value(c)) {
            h.pose_volume(a, Some(v));
        }
    }
}

/// AWarpZoneInfo::execWarp and execUnWarp: Loc through TransformPointBy,
/// Vel through TransformVectorBy and R = OrthoRotation(GMath.UnitCoords
/// / R * C), C being WarpCoords.Transpose()
/// for Warp, WarpCoords for UnWarp.
pub(crate) fn warp_core(warp: &openxiii_assets::coords::Coords, unwarp: bool, loc: V, vel: V, rot: [i32; 3]) -> (V, V, [i32; 3]) {
    use openxiii_assets::coords;
    let c = if unwarp { *warp } else { warp.transpose() };
    let loc = coords::transform_point(loc, &c);
    let vel = coords::transform_vector(vel, &c);
    let r = coords::div_rotator(&coords::UNIT, rot).mul(&c);
    (loc, vel, crate::world::ortho_rotation([r.x, r.y, r.z]))
}

struct HostZoneVm<'a> {
    vm: &'a mut Vm,
}

impl HostZone for HostZoneVm<'_> {
    fn removes(&self, a: ObjId) -> bool {
        self.vm.native_prop(a, Field::DeleteMe).as_bool()
    }
    fn level_value(&self, a: ObjId) -> Option<ObjId> {
        self.vm.native_object(a, Field::Level).or(self.vm.level)
    }
    fn region(&self, a: ObjId) -> Region {
        match self.vm.native_prop(a, Field::Region) {
            Value::Struct(_, f) if f.len() >= 3 => (f[0].as_object(), f[1].as_int(), f[2].as_int() as u8),
            _ => (None, -1, 0),
        }
    }
    fn pose_region(&mut self, a: ObjId, (zone, leaf, number): Region) {
        self.vm.set_region(a, zone, leaf, number);
    }
    fn location(&self, a: ObjId) -> V {
        self.vm.native_prop(a, Field::Location).as_vector()
    }
    fn point_region(&mut self, _level: Option<ObjId>, p: V) -> Region {
        self.vm.point_region(p)
    }
    fn volume_default(&mut self, _level: Option<ObjId>) -> Option<ObjId> {
        self.vm.default_physics_volume()
    }
    fn touching(&self, a: ObjId) -> Vec<Option<ObjId>> {
        self.vm.touching_of(a).into_iter().map(|v| self.vm.objects.get(v).is_some_and(|o| o.is_some()).then_some(v)).collect()
    }
    fn is_volume_physics(&mut self, v: ObjId) -> bool {
        self.vm.is_a(v, "PhysicsVolume")
    }
    fn bounding(&mut self, v: ObjId, p: V) -> bool {
        self.vm.encompasses(v, p)
    }
    fn priority_value(&self, v: ObjId) -> i32 {
        self.vm.priority(v)
    }
    fn volume(&self, a: ObjId) -> Option<ObjId> {
        self.vm.native_object(a, Field::PhysicsVolume)
    }
    fn pose_volume(&mut self, a: ObjId, v: Option<ObjId>) {
        self.vm.set_native_prop(a, Field::PhysicsVolume, Value::Object(v));
    }
    fn head_value(&self, a: ObjId) -> Option<ObjId> {
        self.vm.po(a, "HeadVolume")
    }
    fn pose_head(&mut self, a: ObjId, v: Option<ObjId>) {
        self.vm.set_prop(a, "HeadVolume", Value::Object(v));
    }
    fn height_eyes(&self, a: ObjId) -> f32 {
        self.vm.pf(a, "BaseEyeHeight")
    }
    fn controller(&self, a: ObjId) -> Option<ObjId> {
        self.vm.native_object(a, Field::Controller)
    }
    fn event_value(&mut self, target_value: ObjId, name_value: &'static str, arg: Option<ObjId>) {
        let args = vec![Value::Object(arg)];
        match name_value {
            // AController::eventNotifyPhysicsVolumeChange returns a value.
            "NotifyPhysicsVolumeChange" => {
                self.vm.event_ret(target_value, name_value, args);
            }
            "PawnEnteredVolume" => {
                self.vm.event_pub(target_value, name_value, args);
            }
            _ => self.vm.event_pub(target_value, name_value, args),
        }
    }
}

impl HostVolumes for HostZoneVm<'_> {
    fn actors_list(&self) -> Vec<Option<ObjId>> {
        self.vm.actors.iter().map(|&v| self.vm.objects.get(v).is_some_and(|o| o.is_some()).then_some(v)).collect()
    }
    fn is_volume(&mut self, v: ObjId) -> bool {
        self.vm.is_a(v, "Volume")
    }
    fn collision_actors(&self, a: ObjId) -> bool {
        self.vm.native_prop(a, Field::CollideActors).as_bool()
    }
    fn adds_contact(&mut self, a: ObjId, other_one: ObjId) {
        if let Value::Array(mut items) = self.vm.prop(a, "Touching") {
            items.push(Value::Object(Some(other_one)));
            self.vm.set_prop(a, "Touching", Value::Array(items));
        }
    }
}

/// Water planes of a volume (ULevel::PostLoad): for each polygon
/// of the brush facing up (Normal.Z >= 0.9 in the brush, computed in 24-bit x87 as
/// 1 × Z + 0 × Y + 0 × X) the box of its vertices in the world (ToWorld) gives
/// centre X and Y, highest Z, half-widths X and Y.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub(crate) fn plans_water(polys: &[openxiii_assets::bsp::Poly], to_world: &openxiii_assets::coords::Coords) -> Vec<[f32; 5]> {
    let mut plans = Vec::new();
    for p in polys {
        // (Z × 1 + Y × 0) + X × 0 in single precision; fcomp then "jne" on C0: skipped if the
        // product is less than 0.9 or undefined (NaN).
        let dot = (p.normal[2] * 1.0 + p.normal[1] * 0.0) + p.normal[0] * 0.0;
        if !(dot >= 0.9) {
            continue;
        }
        // FPoly::Vertex is a fixed array: without a vertex, the original reads Vertex[0] (zeros).
        let first = openxiii_assets::coords::transform_point(p.vertices.first().copied().unwrap_or([0.0; 3]), to_world);
        let (mut lo, mut hi) = (first, first);
        for v in p.vertices.iter().skip(1) {
            let w = openxiii_assets::coords::transform_point(*v, to_world);
            for k in 0..3 {
                if hi[k] < w[k] {
                    hi[k] = w[k];
                }
                if w[k] < lo[k] {
                    lo[k] = w[k];
                }
            }
        }
        plans.push([(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, hi[2], (hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5]);
    }
    plans
}

