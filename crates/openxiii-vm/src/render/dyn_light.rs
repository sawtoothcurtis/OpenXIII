//! Dynamic lights of the level (ULevel+0x1118, FLight of 0x44 bytes with the actor at +0x40):
//! ULevel::AddDynLight, RemoveDynLight, RefreshDynLight,
//! called by AActor::execRefreshLighting, and RefreshAllDynLight at the end
//! of UGameEngine::LoadMap. The renderer refreshes each FLight from its actor
//! every frame (FLight::Refresh): `dyn_light_records` does it again.

use crate::NativeField as Field;
use crate::value::{ObjId, Value};
use crate::Vm;
use crate::scene_light_pass::SceneLightId;
use openxiii_assets::coords;

impl Vm {
    /// ULevel::AddDynLight: first free slot (null actor), otherwise one more; a node pointing to
    /// this slot is put at the head of the level's linked list (ULevel+0x113C), even if the actor is already there.
    pub(crate) fn add_dyn_light(&mut self, a: ObjId) {
        let slot = match self.dyn_lights.iter().position(Option::is_none) {
            Some(i) => {
                self.dyn_lights[i] = Some(a);
                i
            }
            None => {
                self.dyn_lights.push(Some(a));
                let i = self.dyn_lights.len() - 1;
                self.forget_dyn_light_slot_cache(i);
                i
            }
        };
        let record = self.dyn_light_record(a);
        self.refresh_dyn_light_slot(slot, record);
        self.dyn_light_list.insert(0, slot);
        self.scene_light_chain.insert(0, SceneLightId::Dynamic(slot));
    }

    /// ULevel::RemoveDynLight: the actor's first slot is freed and the first node of the
    /// list that points to it is removed.
    pub(crate) fn remove_dyn_light(&mut self, a: ObjId) {
        if let Some(i) = self.dyn_lights.iter().position(|s| *s == Some(a)) {
            self.dyn_lights[i] = None;
            if let Some(k) = self.dyn_light_list.iter().position(|&s| s == i) {
                self.dyn_light_list.remove(k);
            }
            if let Some(k) = self.scene_light_chain.iter().position(|id| *id == SceneLightId::Dynamic(i)) {
                self.scene_light_chain.remove(k);
            }
        }
    }

    /// Actor that gives a dynamic light: LightType ≠ LT_None, neither bStatic nor bNoDelete.
    fn dyn_light_actor(&self, a: ObjId) -> bool {
        self.native_prop(a, Field::LightType).as_byte() != 0 && !self.native_prop(a, Field::Static).as_bool() && !self.pb(a, "bNoDelete")
    }

    /// ULevel::RefreshDynLight: removed if the actor no longer gives one, added if it is not there.
    pub(crate) fn refresh_dyn_light(&mut self, a: ObjId) {
        if !self.dyn_light_actor(a) {
            self.remove_dyn_light(a);
            return;
        }
        if !self.dyn_lights.contains(&Some(a)) {
            self.add_dyn_light(a);
        }
    }

    /// ULevel::RefreshAllDynLight: list emptied then RefreshDynLight of each actor.
    pub fn refresh_all_dyn_lights(&mut self) {
        self.scene_light_chain.clear();
        for i in 0..self.scene_model_static_light_count() {
            self.scene_light_chain.insert(0, SceneLightId::Model(i));
        }
        self.dyn_lights.clear();
        self.dyn_light_list.clear();
        self.clear_dyn_light_slot_cache();
        for a in self.actors.clone() {
            if self.alive(a) {
                self.refresh_dyn_light(a);
            }
        }
    }

    /// FLight::Refresh of each dynamic light for this frame: position, direction (X axis of
    /// GMath.UnitCoords / Rotation), light properties, zone, flags (bDecor1Light…
    /// bActorLight, dynamic except bStatic without bDynamicLight, then four bits of the word +0x34).
    pub fn dyn_light_records(&self) -> Vec<openxiii_assets::model::LightRecord> {
        self.dyn_lights.iter().flatten().copied().filter_map(|a| self.dyn_light_record(a)).collect()
    }

    /// Same property read as dyn_light_records, for a single live actor.
    pub(crate) fn dyn_light_record(&self, a: ObjId) -> Option<openxiii_assets::model::LightRecord> {
        if !self.alive(a) {
            return None;
        }
        let b = |n: &str| self.propr(a, n).as_byte();
        let flag = |n: &str| self.pb(a, n) as u32;
        let scenery = [
            "bDecor1Light", "bDecor2Light", "bDecor3Light", "bActorLight", "bDecor4Light", "bDecor5Light", "bDecor6Light", "bDecor7Light",
            "bDecor8Light", "bDecor9Light", "bDecor10Light",
        ];
        let mut flags = scenery.iter().enumerate().fold(0u32, |f, (k, n)| f | flag(n) << k);
        if self.pb(a, "bDynamicLight") || !self.native_prop(a, Field::Static).as_bool() {
            flags |= 0x800;
        }
        // Word +0x34: bIgnoredByShadows (>> 5), bLightOwnZone and bUseOnlyPivotAmbient (>> 6),
        // bLightChanged (>> 7).
        flags |= flag("bIgnoredByShadows") << 12 | flag("bLightOwnZone") << 13 | flag("bUseOnlyPivotAmbient") << 14 | flag("bLightChanged") << 15;
        let zone = match self.native_value(a, Field::Region) {
            Value::Struct(_, f) => f.get(2).map_or(0, |z| z.as_byte()),
            _ => 0,
        };
        Some(openxiii_assets::model::LightRecord {
            location: self.native_prop(a, Field::Location).as_vector(),
            direction: coords::div_rotator(&coords::UNIT, self.native_prop(a, Field::Rotation).as_rotator()).x,
            light_type: b("LightType"),
            effect: b("LightEffect"),
            brightness: b("LightBrightness"),
            hue: b("LightHue"),
            saturation: b("LightSaturation"),
            radius: b("LightRadius"),
            period: b("LightPeriod"),
            phase: b("LightPhase"),
            cone: b("LightCone"),
            zone,
            flags,
        })
    }
}

