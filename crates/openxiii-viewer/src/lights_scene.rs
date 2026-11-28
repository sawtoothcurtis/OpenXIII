//! Light contexts attached to each actual draw, without merging the views.
use std::{collections::HashMap,rc::Rc};
use openxiii_vm::{Vm,value::ObjId,visibility::SceneLightPass};

#[derive(Clone,Debug)]
pub struct Draw {
    pub refresh: Option<Rc<openxiii_vm::visibility::DynRefresh>>,
    pub names: DrawNames,
    pub cull_mode: Option<u8>,
    pub character: Option<usize>,
    pub selection: Option<Rc<openxiii_vm::visibility::SceneLightSelection>>,
}

/// Current name and, when different, alias of the static geometry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DrawNames {
    object: Option<Rc<str>>,
    alias: Option<Rc<str>>,
}

impl DrawNames {
    pub fn iter(&self) -> impl Iterator<Item = &Rc<str>> {
        self.object.iter().chain(&self.alias)
    }
}

#[derive(Clone,Debug,Default)]
pub struct Pass {
    pub sky_zone: Option<i32>,
    /// None: the real ActorScene of Canvas.DrawActor, which keeps the source order.
    pub leaves: Option<Rc<[usize]>>,
    pub lights: Option<Rc<openxiii_vm::scene_light_pass::SceneLights>>,
    pub draws: Rc<[Draw]>,
    /// Number of occurrences of each leaf in `leaves`, counted on first request: without
    /// it, each leaf of each lit actor would walk the whole view list again.
    pub visits: std::cell::OnceCell<TableFast<usize, u32>>,
}

impl Pass {
    /// Same list reader as the three material consumers.
    pub fn leaf_lights(&self,model:&openxiii_assets::model::Model,leaf:usize)->Vec<usize> {
        match &self.leaves {
            Some(leaves)=>{
                let visits = self.visits.get_or_init(|| {
                    let mut t = TableFast::<usize, u32>::default();
                    for &l in leaves.iter() { *t.entry(l).or_default() += 1; }
                    t
                });
                model.scene_leaf_lights_visits(leaf, visits.get(&leaf).copied().unwrap_or(0) as usize)
            }
            None=>model.leaf_lights(leaf),
        }
    }
}

#[derive(Clone,Debug,Default)]
pub struct Views {
    /// False for a geometry viewer without a game VM.
    pub enabled: bool,
    pub main: Rc<IndexedPasses>,
    pub portals: HashMap<usize,Rc<IndexedPasses>>,
    pub targets: HashMap<u64,Rc<IndexedPasses>>,
}

pub type DrawKey = (usize,usize);

type TableFast<K, V> = HashMap<K, V, rustc_hash::FxBuildHasher>;

/// Draws of a name or character: (start, count) in `IndexedPasses::keys`.
type Span = (u32, u32);

#[derive(Debug, Default)]
pub struct IndexedPasses {
    passes: Vec<Pass>,
    /// Table of names and characters, shared with the previous frame if none has changed.
    index: Rc<Index>,
}

#[derive(Debug, Default)]
struct Index {
    /// Draw keys stored name by name then character by character, each in
    /// pass order: a single table rebuilt every frame instead of one vector per name.
    keys: Vec<Option<DrawKey>>,
    by_name: TableFast<Rc<str>, Span>,
    by_character: TableFast<usize, Span>,
}

impl IndexedPasses {
    /// `passes` with the table of `other_one`, built on the same names and characters, rank by rank.
    fn with_index(passes: Vec<Pass>, other_one: &IndexedPasses) -> Rc<Self> {
        Rc::new(Self { passes, index: other_one.index.clone() })
    }

    fn new(passes: Vec<Pass>) -> Rc<Self> {
        let mut index = Index::default();
        let view = &mut index;
        // Distinct names of a draw: the alias is only kept if it differs from the current name.
        fn names(draw: &Draw) -> impl Iterator<Item = &Rc<str>> {
            let mut it = draw.names.iter();
            let first = it.next();
            first.into_iter().chain(it.filter(move |n| Some(*n) != first))
        }
        let count = |range: &mut Span| range.1 += 1;
        // First pass: number of draws per name and per character.
        for pass in passes.iter() {
            for draw in pass.draws.iter() {
                for name in names(draw) {
                    match view.by_name.get_mut(name) {
                        Some(r) => count(r),
                        None => { view.by_name.insert(name.clone(), (0, 1)); }
                    }
                }
                if let Some(character) = draw.character {
                    count(view.by_character.entry(character).or_default());
                }
            }
        }
        let mut next = 0u32;
        for r in view.by_name.values_mut().chain(view.by_character.values_mut()) {
            let len = r.1;
            *r = (next, 0);
            next += len;
        }
        view.keys = vec![None; next as usize];
        // Second pass: keys stored after their range, in pass order.
        let mut put = |keys: &mut Vec<Option<DrawKey>>, r: &mut Span, key| {
            keys[(r.0 + r.1) as usize] = key;
            r.1 += 1;
        };
        for (i, pass) in passes.iter().enumerate() {
            for (j, draw) in pass.draws.iter().enumerate() {
                let key = Some((i, j));
                for name in names(draw) {
                    put(&mut view.keys, view.by_name.get_mut(name).unwrap(), key);
                }
                if let Some(character) = draw.character {
                    put(&mut view.keys, view.by_character.get_mut(&character).unwrap(), key);
                }
            }
        }
        Rc::new(Self { passes, index: Rc::new(index) })
    }

    /// Same DrawKey sequence for each owner and character. Two draws at the
    /// same rank may have different lights and transforms in each view.
    pub fn same_jobs(&self, other: &Self) -> bool {
        // Same table: built on the same names and characters, rank by rank.
        if Rc::ptr_eq(&self.index, &other.index) {
            return true;
        }
        self.passes.len() == other.passes.len() && self.passes.iter().zip(&other.passes).all(|(a, b)| {
            Rc::ptr_eq(&a.draws, &b.draws) || (a.draws.len() == b.draws.len() && a.draws.iter().zip(b.draws.iter())
                .all(|(x, y)| x.character == y.character && x.names == y.names))
        })
    }

    fn slice(&self, r: Option<&Span>) -> &[Option<DrawKey>] {
        r.map_or(&[], |&(start, len)| &self.index.keys[start as usize..(start + len) as usize])
    }
}

impl std::ops::Deref for IndexedPasses {
    type Target = [Pass];
    fn deref(&self) -> &[Pass] { &self.passes }
}

impl Views {
    pub fn name_passes<'a>(&self, passes: &'a IndexedPasses, name: Option<&str>) -> &'a [Option<DrawKey>] {
        let Some(name) = name.filter(|_| self.enabled) else { return &[None] };
        passes.slice(passes.index.by_name.get(name))
    }
    pub fn character_passes<'a>(&self, passes: &'a IndexedPasses, character: usize) -> &'a [Option<DrawKey>] {
        if !self.enabled { return &[None] }
        passes.slice(passes.index.by_character.get(&character))
    }
}

pub fn belongs_to_stage(sky:Option<bool>,sky_stage:bool)->bool {
    sky!=Some(!sky_stage)
}

/// Lowercase names of the actors and aliases of the static geometry, kept from one frame to the next
/// (checked against the current name: an object rank may be taken over by another actor).
#[derive(Default)]
pub struct Names {
    objects_list: TableFast<ObjId,(String,Rc<str>)>,
    alias: TableFast<ObjId,Rc<str>>,
}

impl Names {
    pub fn object_value(&mut self,actor:ObjId,name:&str)->Rc<str> {
        match self.objects_list.get(&actor) {
            Some((n,l)) if n==name => l.clone(),
            _ => {
                let l:Rc<str>=Rc::from(name.to_lowercase());
                self.objects_list.insert(actor,(name.to_string(),l.clone()));
                l
            }
        }
    }
    fn alias(&mut self,actor:ObjId,alias:&str)->Rc<str> {
        match self.alias.get(&actor) {
            Some(a) if &**a==alias => a.clone(),
            _ => {
                let a:Rc<str>=Rc::from(alias);
                self.alias.insert(actor,a.clone());
                a
            }
        }
    }
}

/// Converted VM draws: lowercase names (and static geometry aliases) and character rank.
fn convert_draws<S: std::hash::BuildHasher>(vm:&Vm,rank:&impl Fn(ObjId)->Option<usize>,baked:&HashMap<ObjId,String,S>,names_list:&mut Names,
    source:&[openxiii_vm::visibility::SceneLightDraw])->Rc<[Draw]> {
    source.iter().map(|draw| {
        let actor=draw.actor;
        let object = vm.objects.get(actor).and_then(Option::as_ref).map(|o| names_list.object_value(actor, &o.name));
        let alias = baked.get(&actor).filter(|n| Some(n.as_str()) != object.as_deref()).map(|n| names_list.alias(actor, n));
        let names = DrawNames { object, alias };
        Draw { refresh:draw.refresh.clone(),names,cull_mode:draw.cull_mode,character:rank(actor),selection:draw.selection.clone() }
    }).collect()
}

/// Passes of a view, followed by the Canvas.DrawActor draws (`canvas`, main view) if any.
fn convert_passes<S: std::hash::BuildHasher>(vm:&Vm,rank:&impl Fn(ObjId)->Option<usize>,baked:&HashMap<ObjId,String,S>,names_list:&mut Names,
    source:&[SceneLightPass],canvas:&[openxiii_vm::visibility::SceneLightDraw])->Rc<IndexedPasses> {
    let mut passes:Vec<Pass>=source.iter().map(|pass| Pass {
        sky_zone: pass.sky_zone,
        leaves: pass.leaves.clone(),
        lights: pass.lights.clone(),
        draws: convert_draws(vm, rank, baked, names_list, &pass.draws),
        visits: Default::default(),
    }).collect();
    if !canvas.is_empty() {
        passes.push(Pass { sky_zone: None, leaves: None, lights: None, visits: Default::default(), draws: convert_draws(vm, rank, baked, names_list, canvas) });
    }
    IndexedPasses::new(passes)
}

/// Ranks and names are kept from one frame to the next by the game.
pub fn convert_with<S: std::hash::BuildHasher>(vm:&Vm,rank:impl Fn(ObjId)->Option<usize>,baked:&HashMap<ObjId,String,S>,names_list:&mut Names)->Views {
    Views {
        enabled:true,main:convert_passes(vm, &rank, baked, names_list, &vm.display.scene_light_passes, &vm.display.canvas_source_draws),
        portals:vm.display.portal_light_passes.iter().map(|(&k,v)|(k,convert_passes(vm, &rank, baked, names_list, v, &[]))).collect(),
        targets:vm.display.target_light_passes.iter().map(|(&k,v)|(k,convert_passes(vm, &rank, baked, names_list, v, &[]))).collect(),
    }
}

/// Lists of a view of the previous frame and what they were built from.
struct KeptLights {
    /// VM passes, then the Canvas.DrawActor draws as a last pass if any.
    source: Vec<SceneLightPass>,
    /// Object name of each draw, per pass.
    names_list: Vec<Vec<Option<Box<str>>>>,
    view_value: Rc<IndexedPasses>,
}

/// Same light and leaf cache (shared) and same draws.
fn same_pass(a:&SceneLightPass,b:&SceneLightPass)->bool {
    a.sky_zone == b.sky_zone
        && match (&a.leaves, &b.leaves) { (None, None) => true, (Some(x), Some(y)) => Rc::ptr_eq(x, y) || **x == **y, _ => false }
        && match (&a.lights, &b.lights) { (None, None) => true, (Some(x), Some(y)) => Rc::ptr_eq(x, y), _ => false }
        && a.draws == b.draws
}

fn name_object(vm:&Vm,actor:ObjId)->Option<&str> {
    vm.objects.get(actor).and_then(Option::as_ref).map(|o| o.name.as_str())
}

/// Passes of a view, then the Canvas.DrawActor draws (`canvas`) as a last pass if there
/// are any; the passes unchanged since `previous_one` reuse its draws (compared without copying), the
/// others reuse the names and ranks of the draws of the same actor at the same rank, and the table of names and
/// characters is reused if none of them changed.
fn convert_resume<S: std::hash::BuildHasher>(vm:&Vm,rank:&impl Fn(ObjId)->Option<usize>,baked:&HashMap<ObjId,String,S>,names_list:&mut Names,
    source:&[SceneLightPass],canvas:&[openxiii_vm::visibility::SceneLightDraw],previous_one:Option<KeptLights>)->KeptLights {
    let pass_canvas = (!canvas.is_empty()).then(|| SceneLightPass { lights: None, sky_zone: None, leaves: None, draws: canvas.to_vec() });
    let source: Vec<&SceneLightPass> = source.iter().chain(pass_canvas.as_ref()).collect();
    let names_of = |pass: &SceneLightPass| -> Vec<Option<Box<str>>> { pass.draws.iter().map(|d| name_object(vm, d.actor).map(Into::into)).collect() };
    let same_names_of = |pass: &SceneLightPass, n: &[Option<Box<str>>]| n.len() == pass.draws.len()
        && pass.draws.iter().zip(n).all(|(d, n)| name_object(vm, d.actor) == n.as_deref());
    let Some(mut previous_one) = previous_one else {
        let passes = source.iter().map(|pass| Pass {
            sky_zone: pass.sky_zone, leaves: pass.leaves.clone(), lights: pass.lights.clone(),
            draws: convert_draws(vm, rank, baked, names_list, &pass.draws), visits: Default::default(),
        }).collect();
        return KeptLights { source: source.iter().map(|&p| p.clone()).collect(), names_list: source.iter().map(|p| names_of(p)).collect(), view_value: IndexedPasses::new(passes) };
    };
    let mut same_names = source.len() == previous_one.source.len();
    let mut sources_kept = Vec::with_capacity(source.len());
    let mut names_kept = Vec::with_capacity(source.len());
    let mut passes = Vec::with_capacity(source.len());
    for (k, &pass) in source.iter().enumerate() {
        let s_before = previous_one.source.get_mut(k).map(std::mem::take);
        let n_before = previous_one.names_list.get_mut(k).map(std::mem::take);
        let p_before = previous_one.view_value.passes.get(k);
        let draws = match (s_before, n_before, p_before) {
            (Some(s), Some(n), Some(p)) if same_pass(&s, pass) && same_names_of(pass, &n) => {
                sources_kept.push(s);
                names_kept.push(n);
                p.draws.clone()
            }
            (s, n, p) => {
                let n_new = names_of(pass);
                let draws: Rc<[Draw]> = pass.draws.iter().enumerate().map(|(j, draw)| {
                    let resume = s.as_ref().zip(n.as_ref()).zip(p).and_then(|((s, n), p)| {
                        (s.draws.get(j)?.actor == draw.actor && n.get(j)? == &n_new[j]).then(|| &p.draws[j])
                    });
                    match resume {
                        Some(d) => Draw { refresh: draw.refresh.clone(), names: d.names.clone(), cull_mode: draw.cull_mode, character: d.character, selection: draw.selection.clone() },
                        None => convert_draws(vm, rank, baked, names_list, std::slice::from_ref(draw))[0].clone(),
                    }
                }).collect();
                same_names &= p.is_some_and(|p| p.draws.len() == draws.len()
                    && p.draws.iter().zip(draws.iter()).all(|(x, y)| x.names == y.names && x.character == y.character));
                sources_kept.push(pass.clone());
                names_kept.push(n_new);
                draws
            }
        };
        passes.push(Pass { sky_zone: pass.sky_zone, leaves: pass.leaves.clone(), lights: pass.lights.clone(), draws, visits: Default::default() });
    }
    let view_value = if same_names { IndexedPasses::with_index(passes, &previous_one.view_value) } else { IndexedPasses::new(passes) };
    KeptLights { source: sources_kept, names_list: names_kept, view_value }
}
/// Light lists of the last frame, reused (same Rc) when a view has not changed.
#[derive(Default)]
pub struct KeptLightCache {
    /// Characters (signature of their list) and static geometry of the kept lists.
    key_tuple: Option<(u64, usize)>,
    main: Option<KeptLights>,
    portals: HashMap<usize, KeptLights>,
    targets: HashMap<u64, KeptLights>,
}

/// `convert_with` that reuses the lists of a view from the previous frame when its passes, the
/// names of its actors, the characters (`key_tuple`, signature of their list) and the static geometry
/// are the same: same result, without rebuilding the names or tables.
pub fn convert_kept<S: std::hash::BuildHasher>(vm:&Vm,rank:impl Fn(ObjId)->Option<usize>,baked:&HashMap<ObjId,String,S>,names_list:&mut Names,
    kept_cache:&mut KeptLightCache,key_tuple:u64)->Views {
    if !crate::prop_geometry::kept_active() {
        return convert_with(vm, rank, baked, names_list);
    }
    let key_tuple = Some((key_tuple, baked.len()));
    if kept_cache.key_tuple != key_tuple {
        *kept_cache = KeptLightCache { key_tuple, ..KeptLightCache::default() };
    }
    let mut view_value = |g:&mut Option<KeptLights>,source:&[SceneLightPass],canvas:&[openxiii_vm::visibility::SceneLightDraw]| {
        let new_value = convert_resume(vm, &rank, baked, names_list, source, canvas, g.take());

        let v = new_value.view_value.clone();
        *g = Some(new_value);
        v
    };
    let main = view_value(&mut kept_cache.main, &vm.display.scene_light_passes, &vm.display.canvas_source_draws);
    let mut portals = HashMap::new();
    let mut previous_list = std::mem::take(&mut kept_cache.portals);
    for (&k, v) in &vm.display.portal_light_passes {
        let mut g = previous_list.remove(&k);
        portals.insert(k, view_value(&mut g, v, &[]));
        kept_cache.portals.insert(k, g.unwrap());
    }
    let mut targets = HashMap::new();
    let mut previous_list = std::mem::take(&mut kept_cache.targets);
    for (&k, v) in &vm.display.target_light_passes {
        let mut g = previous_list.remove(&k);
        targets.insert(k, view_value(&mut g, v, &[]));
        kept_cache.targets.insert(k, g.unwrap());
    }
    Views { enabled:true, main, portals, targets }
}
/// Single selection consumed by the Renderer's three material paths.
/// The recursive Render keeps the selected list of the base, not that of the attached actor.
#[cfg(test)]
pub fn selected_lights<'a>(pass:Option<&'a Pass>,draw:Option<&Draw>,own:&openxiii_assets::cartoon::LightTarget,
    _characters:&[Option<openxiii_assets::cartoon::CharacterCartoon>],model:Option<&openxiii_assets::model::Model>,
    lights:&'a [openxiii_assets::lighting::Light])->Vec<&'a openxiii_assets::lighting::Light> {
    pass_lights(pass, draw, model, lights)
        .unwrap_or_else(|| openxiii_assets::cartoon::select_lights(own, lights))
}

/// A Scene pass without a selection has no light. Only the source path without
/// a selection asks the consumer to build its own leaf list.
pub fn pass_lights<'a>(
    pass: Option<&'a Pass>, draw: Option<&Draw>, model: Option<&openxiii_assets::model::Model>,
    lights: &'a [openxiii_assets::lighting::Light],
) -> Option<Vec<&'a openxiii_assets::lighting::Light>> {
    let pass = pass?;
    if let Some(selection) = draw.and_then(|d| d.selection.as_deref()) {
        return Some(scene_selection_lights(pass, selection, model, lights));
    }
    pass.leaves.as_ref().map(|_| Vec::new())
}

/// List of the leaf in the scene context (+0x330, [ebp-0xBC]):
/// Inserts at the head the Model lights that affect the leaf
/// (FLeaf::iPermeating), one copy per call, after the parent node has distributed the dynamic
/// lights. Model indices mapped to those of the snapshot (Model(i)).
fn snapshot_leaf_lights(s:&openxiii_vm::scene_light_pass::SceneLights,pass:&Pass,model:&openxiii_assets::model::Model,leaf:usize)->Vec<usize> {
    use openxiii_vm::scene_light_pass::SceneLightId;
    // Indices mapped in place: a single vector per leaf.
    let mut out:Vec<usize>=pass.leaf_lights(model,leaf);
    let mut kept=0;
    for k in 0..out.len() {
        if let Some(i)=s.index_of(SceneLightId::Model(out[k])) { out[kept]=i; kept+=1; }
    }
    out.truncate(kept);
    out.extend(s.leaves.lights(leaf));
    out
}

/// Cached parameters of the Scene base, independent of any loaded geometry.
pub fn scene_selection_lights<'a>(pass:&'a Pass,selection:&openxiii_vm::visibility::SceneLightSelection,
    model:Option<&openxiii_assets::model::Model>,lights:&'a [openxiii_assets::lighting::Light])->Vec<&'a openxiii_assets::lighting::Light> {
    use openxiii_assets::cartoon::{select_lights,select_distributed_lights,LightTarget};
    // The cache belongs to this Scene draw. Source keeps exactly its existing
    // reader/order and its legacy entry; no extra Init.
    let snapshot=pass.lights.as_deref().filter(|_|pass.leaves.is_some());
    let lights=snapshot.map_or(lights,|s|s.lights.as_slice());
    let mut zones=Vec::new();let mut lists=Vec::new();
    // No light for bUnlit nor, in the game, bIgnoreDynLight.
    if let (Some(model),false)=(model,selection.unlit || selection.ignore_dyn) {
        for &leaf in &selection.leaves {
            if let Some(&zone)=model.leaves.get(leaf as usize) {
                zones.push(zone);
                lists.push(match snapshot {
                    Some(s)=>snapshot_leaf_lights(s,pass,model,leaf as usize),
                    None=>pass.leaf_lights(model,leaf as usize),
                });
            }
        }
    }
    let target=LightTarget {light_group:selection.light_group,leaf_zones:&zones,leaf_lights:&lists,sphere:selection.sphere};
    if snapshot.is_some() { select_distributed_lights(&target,lights) }
    else if pass.leaves.is_some() { select_lights(&target,lights) }
    else { openxiii_assets::cartoon::select_actor_scene_lights(&target,lights) }
}

/// Existing path of the animated Renderer, kept without changing its Source contract.
pub fn character_lights<'a>(pass:Option<&Pass>,character:&openxiii_assets::cartoon::CharacterCartoon,
    model:Option<&openxiii_assets::model::Model>,lights:&'a [openxiii_assets::lighting::Light])->Vec<&'a openxiii_assets::lighting::Light> {
    use openxiii_assets::cartoon::{select_lights,LightTarget};
    let mut zones=Vec::new();let mut lists=Vec::new();
    if let (Some(model),false)=(model,character.unlit) {
        for &leaf in &character.leaves {
            if let Some(&zone)=model.leaves.get(leaf as usize) {
                zones.push(zone);
                lists.push(match pass { Some(p)=>p.leaf_lights(model,leaf as usize),None=>model.leaf_lights(leaf as usize) });
            }
        }
    }
    select_lights(&LightTarget { light_group:character.light_group,leaf_zones:&zones,leaf_lights:&lists,sphere:character.sphere },lights)
}

pub fn draw_character_lights<'a>(pass:Option<&'a Pass>,draw:Option<&Draw>,own:&openxiii_assets::cartoon::CharacterCartoon,
    _characters:&[Option<openxiii_assets::cartoon::CharacterCartoon>],model:Option<&openxiii_assets::model::Model>,
    lights:&'a [openxiii_assets::lighting::Light])->Vec<&'a openxiii_assets::lighting::Light> {
    pass_lights(pass, draw, model, lights)
        .unwrap_or_else(|| character_lights(pass, own, model, lights))
}

