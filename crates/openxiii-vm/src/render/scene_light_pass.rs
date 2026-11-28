//! State of the light-only sub-path of Scene.
//! The real Model's records are read at load time. A missing Model cache is
//! kept as such: only the light prerequisite fills it.

pub use crate::scene_light_distribution::LightLists;
use crate::{Vm,visibility::SceneNode};
use openxiii_assets::{lighting::{self,Light},model::LightRecord};

#[derive(Clone,Copy,Debug,PartialEq,Eq,Hash,PartialOrd,Ord)]
pub enum SceneLightId { Model(usize), Dynamic(usize) }

#[derive(Clone,Debug,Default)]
pub struct SceneLights {
    pub ids: Vec<SceneLightId>,
    pub lights: Vec<Light>,
    pub leaves: LightLists,
    pub surfaces: LightLists,
    /// Explicit limit: no PostLoad cache has been carried for these records.
    pub unavailable_model: Vec<usize>,
    /// Diagnostic: the Model metadata exists but its real chain is missing.
    pub unavailable_model_chain: bool,
}

impl SceneLights {
    /// The Model then Dynamic slots are published in order, without the NULLs.
    pub fn index_of(&self, id: SceneLightId) -> Option<usize> {
        if let SceneLightId::Model(i) = id {
            if self.ids.get(i) == Some(&id) { return Some(i); }
        }
        self.ids.binary_search(&id).ok()
    }
}

fn same_light(a:&Light,b:&Light)->bool {
    a.color.map(f32::to_bits)==b.color.map(f32::to_bits)
    &&a.location.map(f32::to_bits)==b.location.map(f32::to_bits)
    &&a.direction.map(f32::to_bits)==b.direction.map(f32::to_bits)
    &&a.radius.to_bits()==b.radius.to_bits()&&a.dynamic==b.dynamic&&a.animated==b.animated
    &&a.light_type==b.light_type&&a.effect==b.effect&&a.period==b.period&&a.phase==b.phase
    &&a.effect_time.to_bits()==b.effect_time.to_bits()&&a.brightness==b.brightness
    &&a.cone==b.cone&&a.zone==b.zone&&a.flags==b.flags
}
impl PartialEq for SceneLights {
    fn eq(&self,other:&Self)->bool {
        self.ids==other.ids&&self.leaves==other.leaves&&self.surfaces==other.surfaces
        &&self.unavailable_model==other.unavailable_model&&self.unavailable_model_chain==other.unavailable_model_chain&&self.lights.len()==other.lights.len()
        &&self.lights.iter().zip(&other.lights).all(|(a,b)|same_light(a,b))
    }
}

#[derive(Default)]
pub(crate) struct SceneLightModel {
    /// A map bind took place, even if this Model is really empty.
    pub bound: bool,
    pub key: Option<(usize,usize)>,
    pub records: Vec<LightRecord>,
    pub cache: Vec<Option<Light>>,
    pub static_count: usize,
    pub refresh_from_actors: bool,
    pub node_lights: LightLists,
}

impl Vm {
    /// At the real Model metadata producer of load_brushes. This does not initialize
    /// the FLights: it consumes the result of the PostLoad prerequisite.
    pub fn scene_light_model_loaded(&mut self,key:Option<(usize,usize)>,records:Vec<LightRecord>,static_count:usize,refresh:bool) {
        self.clear_dyn_light_slot_cache();
        self.scene_light_chain.clear();
        let cache=key.and_then(|(pi,export)|self.post_loaded_model_lights(pi,export)).map(|lights|{
            assert_eq!(lights.len(),records.len(),"Model cache and loaded data from two different generations");
            lights.iter().cloned().map(Some).collect()
        }).unwrap_or_else(||vec![None;records.len()]);
        self.scene_light_model=SceneLightModel {
            bound:true,key,cache,records,static_count,refresh_from_actors:refresh,node_lights:LightLists::default(),
        };
    }

    /// Output of the real ModelPostLoad: bit-for-bit copy, no Init or RNG here.
    pub(crate) fn scene_model_postloaded(&mut self,key:(usize,usize),lights:Vec<Light>) {
        if self.scene_light_model.key==Some(key) {
            self.scene_light_model.cache=lights.iter().cloned().map(Some).collect();
        }
        self.model_light_caches.insert(key,lights);
    }

    /// Result of the real PostLoad, also consumed by the load_brushes bind.
    pub fn post_loaded_model_lights(&self,package:usize,export:usize)->Option<&[Light]> {
        self.model_light_caches.get(&(package,export)).map(Vec::as_slice)
    }

    /// Count of the current real Model, for RefreshAll.
    pub(crate) fn scene_model_static_light_count(&self)->usize {
        self.scene_light_model.static_count.min(self.scene_light_model.records.len())
    }

    /// Model cache from the PostLoad, then from the real Scene updates.
    pub fn scene_model_light_cache(&self)->&[Option<Light>] {
        &self.scene_light_model.cache
    }

    fn scene_init_model_record(&mut self,index:usize,record:LightRecord)->usize {
        let keep_animated=record.light_type!=1&&self.scene_light_model.cache[index].as_ref().is_some_and(|l|l.animated);
        let brightness=self.level.map_or(1.0,|l|self.propr(l,"Brightness").as_float());
        let time=self.level_time;let mut draws=0;
        let mut light=lighting::init_light(&record,brightness,time,&mut||{draws+=1;self.rand()});
        light.animated|=keep_animated;
        if let Some(cache)=self.scene_light_model.key.and_then(|key|self.model_light_caches.get_mut(&key)) {
            cache[index]=light.clone();
        }
        self.scene_light_model.cache[index]=Some(light);draws
    }

    /// Update at most once per frame, then choice
    /// of this view's own chain. The second view mode134=1 stays empty.
    pub(crate) fn prepare_scene_lights(&mut self,node:&SceneNode)->(SceneLights,Vec<usize>,usize) {
        let mut local=Vec::new();let mut draws=0;
        let update=self.engine_frames!=self.lights_frame;
        if update {
            self.lights_frame=self.engine_frames;
            let eligible=node.viewer().is_some_and(|viewer|{
                let map=self.propr(viewer,"RendMap").as_int();
                map==5||map==7||(map==16&&self.propr(viewer,"Misc1").as_int()!=0)
            });
            if eligible {
                if self.scene_light_model.refresh_from_actors {
                    let count=self.scene_light_model.records.len().min(self.light_actor_cases.len());
                    for index in 0..count {
                        if let Some(record)=self.light_actor_cases[index].and_then(|actor|self.dyn_light_record(actor)) {
                            draws+=self.scene_init_model_record(index,record);
                        } else if let Some(light)=self.scene_light_model.cache[index].as_mut() {
                            // NULL keeps its metadata and becomes animated black.
                            light.color=[0.;4];light.animated=true;
                            if let Some(cache)=self.scene_light_model.key.and_then(|key|self.model_light_caches.get_mut(&key)) {
                                cache[index]=light.clone();
                            }
                        }
                        if self.scene_light_model.cache[index].as_ref().is_some_and(|l|l.dynamic||l.animated) {
                            local.push(SceneLightId::Model(index));
                        }
                    }
                } else {
                    for index in 0..self.scene_light_model.static_count.min(self.scene_light_model.records.len()) {
                        draws+=self.scene_init_model_record(index,self.scene_light_model.records[index].clone());
                    }
                }
                for slot in 0..self.dyn_lights.len() {
                    let record=self.dyn_lights[slot].and_then(|actor|self.dyn_light_record(actor));
                    draws+=self.refresh_dyn_light_slot(slot,record);
                    if self.scene_light_model.refresh_from_actors { local.push(SceneLightId::Dynamic(slot)); }
                }
            }
        }
        let count=self.scene_light_model.cache.len()+self.dyn_light_slot_cache().len();
        let mut snapshot=SceneLights { ids:Vec::with_capacity(count),lights:Vec::with_capacity(count),..Default::default() };
        for (index,cache) in self.scene_light_model.cache.iter().enumerate() {
            if let Some(light)=cache {
                snapshot.ids.push(SceneLightId::Model(index));snapshot.lights.push(light.clone());
            } else { snapshot.unavailable_model.push(index); }
        }
        for (slot,cache) in self.dyn_light_slot_cache().iter().enumerate() {
            if let Some(light)=cache {
                snapshot.ids.push(SceneLightId::Dynamic(slot));snapshot.lights.push(light.clone());
            }
        }
        let chain=if self.scene_light_model.refresh_from_actors { local.reverse(); &local }else{
            // Read the chain actually produced by RefreshAll/Add/Remove.
            // No Model->reverse rebuild is done in Scene.
            snapshot.unavailable_model_chain=self.scene_model_static_light_count()!=0
                &&!self.scene_light_chain.iter().any(|id|matches!(id,SceneLightId::Model(_)));
            &self.scene_light_chain
        };
        let root=chain.iter().filter_map(|key|snapshot.index_of(*key)).collect();
        (snapshot,root,draws)
    }
}

