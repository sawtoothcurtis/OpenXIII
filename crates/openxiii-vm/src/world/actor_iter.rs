//! Destruction of an actor (ULevel::DestroyActor, AActor::execDestroy) and
//! actor iterators walked during the loop as the engine does: AllActors,
//! DynamicActors, ChildActors, BasedActors,
//! TouchingActors, RadiusActors, VisibleActors,
//! ZoneActors, ActorInIterationCategory, VisibleCollidingActors
//! and VisibleDamageableActors.
//!
//! The engine reads XLevel->Actors index by index during the foreach: an actor created by the
//! loop body is visited, a destroyed actor leaves a hole (Actors(i) = NULL). Here the list
//! `actors` loses the destroyed element; each removal is recorded (`actor_removals`) and the
//! cursors in progress move back accordingly, which amounts to the same traversal.

use crate::physics::CheckResult;
use crate::value::{ClassId, ObjId, Value};
use crate::types::Type;
use crate::Vm;

type V = [f32; 3];

/// Actor iterator in progress.
#[derive(Debug, Clone)]
pub(crate) struct LazyIter {
    kind: Kind,
    class: Option<ClassId>,
    /// Next index to read (in `actors`, or in Touching).
    cursor: usize,
    /// Removals of `actor_removals` already taken into account.
    seen_removals: usize,
}

#[derive(Debug, Clone)]
enum Kind {
    /// AllActors / DynamicActors (the latter from iFirstDynamicActor), optional Tag.
    All(String),
    /// Actors whose chain of Owners, the actor included, leads to `this`.
    Child(ObjId),
    Based(ObjId),
    Touching(ObjId),
    /// (dz² + dy²) + dx² < (Radius + CollisionRadius)².
    Radius(f32, V),
    /// Not hidden, Radius zero or distance < Radius, trace 0x286 from the actor to Loc.
    Visible(ObjId, f32, V),
    /// Region.Zone of the actor equal to its Level (default zone) or to zone `this`.
    Zone(ObjId),
    /// IterationCategory equal to Min (Min = Max) or between Min and Max, without a class filter, from
    /// the end of XLevel->Actors towards the start; `cursor` is the index after the next one read.
    Category(u8, u8),
    /// VisibleCollidingActors / VisibleDamageableActors: octree list taken at the start of the
    /// loop, each candidate tested (`visible_radius_ok`) when the loop reaches it; `cursor`
    /// index in the list.
    VisibleRadius { damageable: bool, this: ObjId, loc: V, ignore_hidden: bool, list: Vec<ObjId> },
}

impl Vm {
    pub(crate) fn lazy_iterator(&self, name: &str, this: ObjId, class: Option<ClassId>, tag: String, radius: f32, loc: V) -> Option<LazyIter> {
        // AActor::execDynamicActors starts from ULevel::iFirstDynamicActor.
        let cursor = if name == "DynamicActors" { self.first_dynamic.min(self.actors.len()) } else { 0 };
        let kind = match name {
            "AllActors" | "DynamicActors" => Kind::All(tag),
            "ChildActors" => Kind::Child(this),
            "BasedActors" => Kind::Based(this),
            "TouchingActors" => Kind::Touching(this),
            "RadiusActors" => Kind::Radius(radius, loc),
            "VisibleActors" => Kind::Visible(this, radius, loc),
            "ZoneActors" => Kind::Zone(this),
            _ => return None,
        };
        // Class None: Actor.
        let class = class.or_else(|| self.class("Actor"));
        Some(LazyIter { kind, class, cursor, seen_removals: self.actor_removals.len() })
    }

    /// VisibleCollidingActors / VisibleDamageableActors: the octree at the
    /// start of the loop, visibility at each iteration.
    pub(crate) fn visible_radius_iterator(&mut self, name: &str, this: ObjId, class: Option<ClassId>, radius: f32, loc: V, ignore_hidden: bool) -> LazyIter {
        let list = self.visible_radius_candidates(name, this, loc, radius);
        let kind = Kind::VisibleRadius { damageable: name == "VisibleDamageableActors", this, loc, ignore_hidden, list };
        LazyIter { kind, class, cursor: 0, seen_removals: self.actor_removals.len() }
    }

    /// AActor::execActorInIterationCategory: bounds put back in order, backwards
    /// traversal from the last actor present at the start of the loop.
    pub(crate) fn category_iterator(&self, min: u8, max: u8) -> LazyIter {
        let (lo, hi) = if max < min { (max, min) } else { (min, max) };
        LazyIter { kind: Kind::Category(lo, hi), class: None, cursor: self.actors.len(), seen_removals: self.actor_removals.len() }
    }

    /// Next element of the iterator (None: end of the loop).
    pub(crate) fn lazy_next(&mut self, it: &mut LazyIter) -> Option<ObjId> {
        if let Kind::VisibleRadius { damageable, this, loc, ignore_hidden, list } = &it.kind {
            let (damageable, this, loc, ignore_hidden) = (*damageable, *this, *loc, *ignore_hidden);
            while let Some(&o) = list.get(it.cursor) {
                it.cursor += 1;
                if self.visible_radius_ok(damageable, this, it.class, loc, ignore_hidden, o) {
                    return Some(o);
                }
            }
            return None;
        }
        if !matches!(it.kind, Kind::Touching(_)) {
            for &r in &self.actor_removals[it.seen_removals..] {
                if r < it.cursor {
                    it.cursor -= 1;
                }
            }
            it.seen_removals = self.actor_removals.len();
        }
        if let Kind::Category(lo, hi) = it.kind {
            while it.cursor > 0 {
                it.cursor -= 1;
                let Some(&o) = self.actors.get(it.cursor) else { continue };
                if self.objects.get(o).is_none_or(|x| x.is_none()) {
                    continue;
                }
                let c = self.propr(o, "IterationCategory").as_byte();
                if if lo == hi { c == lo } else { lo <= c && c <= hi } {
                    return Some(o);
                }
            }
            return None;
        }
        let is_a = |vm: &Vm, o: ObjId| vm.objects.get(o).and_then(|x| x.as_ref()).is_some_and(|x| it.class.is_some_and(|c| vm.is_child_of(x.class, c)));
        if let Kind::Touching(this) = it.kind {
            let touching = self.touching_of(this);
            while let Some(&o) = touching.get(it.cursor) {
                it.cursor += 1;
                if is_a(self, o) {
                    return Some(o);
                }
            }
            return None;
        }
        // TriggerEvent walks DynamicActors with a name. Reading the borrowed Tag
        // first avoids copying each name and walking up the class of the other actors.
        // The cursor stays the level's: the foreach body can change it.
        if let Kind::All(tag) = &it.kind {
            if !tag.is_empty() {
                while let Some(&o) = self.actors.get(it.cursor) {
                    it.cursor += 1;
                    if self.alive(o) && self.native_prop(o, crate::NativeField::Tag).name_ref().eq_ignore_ascii_case(tag) && is_a(self, o) {
                        return Some(o);
                    }
                }
                return None;
            }
        }
        while let Some(&o) = self.actors.get(it.cursor) {
            it.cursor += 1;
            if !is_a(self, o) {
                continue;
            }
            let ok = match &it.kind {
                Kind::All(_) => true,
                Kind::Child(this) => {
                    let mut a = Some(o);
                    let mut found = false;
                    for _ in 0..256 {
                        match a {
                            Some(x) if x == *this => {
                                found = true;
                                break;
                            }
                            Some(x) => a = self.propr(x, "Owner").as_object(),
                            None => break,
                        }
                    }
                    found
                }
                Kind::Based(this) => self.propr(o, "Base").as_object() == Some(*this),
                Kind::Touching(_) | Kind::Category(..) | Kind::VisibleRadius { .. } => false,
                Kind::Zone(this) => {
                    let zone = match self.prop(o, "Region") {
                        Value::Struct(_, f) => f.first().and_then(|z| z.as_object()),
                        _ => None,
                    };
                    zone == self.propr(o, "Level").as_object() || zone == Some(*this)
                }
                Kind::Radius(radius, loc) => {
                    let l = self.propr(o, "Location").as_vector();
                    let d = [l[0] - loc[0], l[1] - loc[1], l[2] - loc[2]];
                    let r = radius + self.propr(o, "CollisionRadius").as_float();
                    (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0] < r * r
                }
                Kind::Visible(this, radius, loc) => {
                    let l = self.propr(o, "Location").as_vector();
                    let d = [l[0] - loc[0], l[1] - loc[1], l[2] - loc[2]];
                    !self.propr(o, "bHidden").as_bool()
                        && (*radius == 0.0 || (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0] < radius * radius)
                        && {
                            let mut hit = CheckResult::new(1.0);
                            self.single_line_check(&mut hit, Some(*this), l, *loc, 0x286, [0.0; 3]);
                            hit.actor.is_none() || hit.actor == Some(o)
                        }
                }
            };
            if ok {
                return Some(o);
            }
        }
        None
    }

    /// ULevel::DestroyActor, outside the editor, bNetForce zero. False if the actor cannot
    /// be destroyed (bStatic, bNoDelete, or not the authority without bNetTemporary).
    pub(crate) fn destroy_actor(&mut self, a: ObjId) -> crate::Result<bool> {
        self.destroy_actor_net(a, false)
    }

    /// ULevel::DestroyActor(Actor, bNetForce): bNetForce also destroys a replicated actor
    /// (UActorChannel::Destroy on the client).
    pub(crate) fn destroy_actor_net(&mut self, a: ObjId, net_force: bool) -> crate::Result<bool> {
        self.destroy_actor_full(a, net_force, false)
    }

    /// ULevel::DestroyActor(Actor, bNetForce, bEditorOnly): with the third parameter, the original skips the refusals at the start (bStatic, bNoDelete,
    /// already bDeleteMe, non-authoritative role) — this is the call from CleanLightActors (DestroyActor(actor, 0, 1)).
    pub(crate) fn destroy_actor_full(&mut self, a: ObjId, net_force: bool, editor_only: bool) -> crate::Result<bool> {
        if self.objects.get(a).is_none_or(|o| o.is_none()) {
            return Ok(true);
        }
        let deleted = |vm: &Vm| vm.objects.get(a).is_none_or(|o| o.is_none()) || vm.propr(a, "bDeleteMe").as_bool();
        if !editor_only {
            if self.propr(a, "bStatic").as_bool() || self.propr(a, "bNoDelete").as_bool() {
                return Ok(false);
            }
            if deleted(self) {
                return Ok(true);
            }
            if self.propr(a, "Role").as_byte() != 4 && !net_force && !self.propr(a, "bNetTemporary").as_bool() {
                return Ok(false);
            }
        }
        self.set_prop(a, "bPendingDelete", Value::Bool(true));
        // Current state: EndState.
        if self.objects[a].as_ref().is_some_and(|o| o.state.is_some()) {
            self.event_pub(a, "EndState", vec![]);
            if deleted(self) {
                return Ok(true);
            }
        }
        // SetBase(NULL), then the attached actors lose their support.
        if self.propr(a, "Base").as_object().is_some() {
            self.set_base_full(a, None, [0.0, 0.0, 1.0], true);
            if deleted(self) {
                return Ok(true);
            }
        }
        // for (i = 0; i < Attached.Num(); i++) if (Attached(i)) Attached(i)->SetBase(NULL, (0,0,1), 1):
        // SetBase removes the actor from the Attached array, the index still advances and the next one is skipped.
        let mut i = 0;
        loop {
            let Value::Array(attached) = self.prop(a, "Attached") else { break };
            let Some(v) = attached.get(i) else { break };
            if let Some(o) = v.as_object().filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) {
                self.set_base_full(o, None, [0.0, 0.0, 1.0], true);
            }
            i += 1;
        }
        if let Some(f) = self.find_function(a, "Destroyed", false) {
            if self.is_probing(a, "Destroyed") {
                self.call(f, a, vec![])?;
            }
        }
        if self.ipdrv.udp.contains_key(&a) {
            self.udp_link_destroyed(a);
        }
        if deleted(self) {
            return Ok(true);
        }
        // The actors it owns lose their owner; its contacts end.
        let mut i = 0;
        while let Some(&o) = self.actors.get(i) {
            i += 1;
            // Owner through its native field (without looking up the name); a freed actor keeps the read by name.
            let owner = if self.objects.get(o).is_some_and(|x| x.is_some()) { self.native_prop(o, crate::NativeField::Owner).as_object() } else { self.propr(o, "Owner").as_object() };
            if owner == Some(a) {
                self.set_owner(o, None);
                if deleted(self) {
                    return Ok(true);
                }
            }
            // Touching.FindItem of the other actor (this = o + 0xB4, looks for the destroyed actor),
            // read in place without copying the list (through its native field, like Owner).
            let touching = if self.objects.get(o).is_some_and(|x| x.is_some()) { self.native_prop(o, crate::NativeField::Touching) } else { self.propr(o, "Touching") };
            if matches!(touching, Value::Array(items) if items.iter().any(|v| v.as_object() == Some(a))) {
                self.end_touch(a, o, true);
                if deleted(self) {
                    return Ok(true);
                }
            }
        }
        if let Some(owner) = self.propr(a, "Owner").as_object().filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) {
            self.event_pub(owner, "LostChild", vec![Value::Object(Some(a))]);
            if deleted(self) {
                return Ok(true);
            }
        }
        // UNetDriver::NotifyActorDestroyed (0x103891xx): its actor channels close.
        if self.ipdrv.net_driver.is_some() {
            self.net_notify_actor_destroyed(a);
        }
        if self.native_prop(a, crate::NativeField::CollideActors).as_bool() {
            self.octree_remove(a);
        }
        self.remove_dyn_disp(a, None);
        self.remove_dyn_light(a);
        // ULevel::GetActorIndex: first index of the actor in Actors.
        if let Some(pos) = self.actors.iter().position(|&x| x == a) {
            self.actors.remove(pos);
            self.actors_changed();
            if pos < self.first_dynamic {
                self.first_dynamic -= 1;
            }
            self.actor_removals.push(pos);
        }
        self.set_prop(a, "bDeleteMe", Value::Bool(true));
        // The editor call (bEditorOnly) does not chain the actor into the list of destroyed actors (FirstDeleted).
        if !editor_only {
            self.first_deleted.push(a);
        }
        self.threads.remove(&a);
        self.audio_note_destroy(a);
        // AProjector::Destroy: DetachProjector(1); ATrail::Destroy:
        // vertex buffer freed.
        self.projector_destroyed(a);
        self.trails.remove(&a);
        self.emitter_destroyed(a);
        // AActor::Destroy: the actor leaves the octree if it is still there, its FOctreeInfo is freed.
        // AShadowProjector::Destroy calls UObject::Destroy without going through AActor::Destroy: neither one nor the other.
        if !self.is_a(a, "ShadowProjector") {
            // AActor::Destroy first empties Actor.Projectors (+0x128).
            self.actor_projectors_destroyed(a);
            self.octree_destroy(a);
        }
        self.exit_properties(a);
        Ok(true)
    }

    /// AEmitter::Destroy, outside the editor: each ParticleEmitter of the Emitters list
    /// is deleted (vt 0x10, destructor: UObject::ConditionalDestroy → UParticleEmitter::Destroy
    ///CleanUp then UObject::Destroy, ExitProperties) and its entry set to
    /// NULL. An Emitter constructed in game has its own copies (`construct_emitters`); the VM keeps
    /// the object as Destroy left it, for any references that remain, and forgets its state.
    fn emitter_destroyed(&mut self, a: ObjId) {
        if !self.is_a(a, "Emitter") {
            return;
        }
        let Value::Array(mut items) = self.prop(a, "Emitters") else { return };
        for v in items.iter_mut() {
            let Some(e) = v.as_object().filter(|&e| self.objects.get(e).is_some_and(|o| o.is_some())) else { continue };
            self.particles.states.remove(&e);
            self.exit_properties(e);
            *v = Value::Object(None);
        }
        self.set_prop(a, "Emitters", Value::Array(items));
    }

    /// UObject::ExitProperties: DestroyValue (vt 0xB0) of each property.
    fn exit_properties(&mut self, a: ObjId) {
        let class = self.objects[a].as_ref().map(|o| o.class);
        let Some(c) = class else { return };
        let defs: Vec<(Type, usize)> = self.classes[c].props.iter().map(|d| (d.ty.clone(), d.dim)).collect();
        let Some(mut props) = self.objects[a].as_mut().map(|o| std::mem::take(&mut o.props)) else { return };
        props.with_slice(|v| self.exit_property_values(v, &defs));
        self.objects[a].as_mut().unwrap().props = props;
    }

    /// Body of ExitProperties: DestroyValue of each property; only those of
    /// the original's ConstructorLink chain (strings, arrays, structs that contain them) change.
    pub(crate) fn exit_property_values(&mut self, props: &mut [Value], defs: &[(Type, usize)]) {
        for (v, (ty, dim)) in props.iter_mut().zip(defs) {
            self.destroy_value(v, ty, *dim);
        }
    }

    /// DestroyValue of the properties: UStrProperty empties the string,
    /// UArrayProperty destroys its elements then frees the array (0 elements),
    /// UStructProperty destroys the fields of the struct; the others are not
    /// in the ConstructorLink chain.
    fn destroy_value(&mut self, v: &mut Value, ty: &Type, dim: usize) {
        if dim > 1 {
            if let Value::Array(items) = v {
                items.iter_mut().for_each(|x| self.destroy_value(x, ty, 1));
            }
            return;
        }
        match ty {
            Type::Str => *v = Value::Str(String::new()),
            Type::Array(_) => *v = Value::Array(Vec::new()),
            Type::Struct(name) => {
                let fields: Vec<(Type, usize)> = self.struct_def(name).map(|s| s.fields.iter().map(|f| (f.ty.clone(), f.dim)).collect()).unwrap_or_default();
                if let Value::Struct(_, values) = v {
                    for (x, (t, d)) in values.iter_mut().zip(&fields) {
                        self.destroy_value(x, t, *d);
                    }
                }
            }
            _ => {}
        }
    }
}

