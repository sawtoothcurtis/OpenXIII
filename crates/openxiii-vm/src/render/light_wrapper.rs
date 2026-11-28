//! State of the FLight wrapper, called by the real Add and Scene.
//! The Scene producer calls Init once per eligible frame.
use crate::Vm;
use openxiii_assets::{lighting::{self,Light},model::LightRecord};

impl Vm {
    /// A slot with an actor calls the real Init from its property record.
    /// The NULL slot only resets RGBA at +0, without inventing metadata.
    /// Returns the number of real RNG calls, for observing the Scene producer.
    pub(crate) fn refresh_dyn_light_slot(&mut self, slot:usize, record:Option<LightRecord>) -> usize {
        assert!(slot<self.dyn_lights.len(), "light slot outside the level's array");
        self.dyn_light_cache.truncate(self.dyn_lights.len());
        self.dyn_light_cache.resize_with(self.dyn_lights.len(),||None);
        match record {
            Some(record)=>{
                // Init only resets +31 to zero for Type1. The wrapper carries
                // the bytes of the existing slot, without changing the numeric callee.
                let keep_animated=record.light_type!=1&&self.dyn_light_cache[slot].as_ref().is_some_and(|l|l.animated);
                let brightness=self.level.map_or(1.0,|level|self.propr(level,"Brightness").as_float());
                let time=self.level_time;
                let mut draws=0;
                let mut light=lighting::init_light(&record,brightness,time,&mut||{
                    draws+=1;
                    self.rand()
                });
                light.animated|=keep_animated;
                self.dyn_light_cache[slot]=Some(light);
                draws
            }
            None=>{
                if let Some(light)=self.dyn_light_cache[slot].as_mut() {
                    light.color=[0.0;4];
                }
                0
            }
        }
    }

    /// The bound is always that of the live slots, even between RefreshAll.clear
    /// and the new Adds. Older passes keep their own Rc copy.
    pub(crate) fn dyn_light_slot_cache(&self)->&[Option<Light>] {
        &self.dyn_light_cache[..self.dyn_light_cache.len().min(self.dyn_lights.len())]
    }

    /// On binding the map metadata and on the real RefreshAll, even at equal size.
    pub(crate) fn clear_dyn_light_slot_cache(&mut self) {
        self.dyn_light_cache.clear();
    }

    /// Only the Add branch that builds a new FLight after a push.
    /// Reusing a NULL slot must not come through here. No Init and no RNG.
    pub(crate) fn forget_dyn_light_slot_cache(&mut self,slot:usize) {
        assert!(slot<self.dyn_lights.len());
        self.dyn_light_cache.truncate(self.dyn_lights.len());
        self.dyn_light_cache.resize_with(self.dyn_lights.len(),||None);
        self.dyn_light_cache[slot]=None;
    }
}
