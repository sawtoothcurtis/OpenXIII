//! CPU geometry of moving props. The cache belongs to a Characters list,
//! whose models and actor-to-rank mapping are immutable.
use openxiii_assets::level::MeshActor;
use std::rc::Rc;

pub fn kept_active() -> bool {
    true
}

/// Positions and normals computed on first request.
type Deferred = Box<dyn Fn(&mut Vec<[f32; 3]>, &mut Vec<[f32; 3]>)>;

pub struct WorldGeometry {
    pub min: [f32; 3],
    pub max: [f32; 3],
    len: usize,
    vertices_list: std::cell::OnceCell<(Vec<[f32; 3]>, Vec<[f32; 3]>)>,
    deferred: Option<Deferred>,
}

impl std::fmt::Debug for WorldGeometry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldGeometry").field("min", &self.min).field("max", &self.max).field("len", &self.len).finish()
    }
}

impl WorldGeometry {
    pub fn new(vertices: impl ExactSizeIterator<Item = ([f32; 3], [f32; 3])>) -> Self {
        let (mut positions, mut normals) = (Vec::with_capacity(vertices.len()), Vec::with_capacity(vertices.len()));
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (position, normal) in vertices {
            for k in 0..3 {
                min[k] = min[k].min(position[k]);
                max[k] = max[k].max(position[k]);
            }
            positions.push(position);
            normals.push(normal);
        }
        Self { min, max, len: positions.len(), vertices_list: std::cell::OnceCell::from((positions, normals)), deferred: None }
    }

    /// World positions of the vertices (computed on first request).
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.vertices_list().0
    }

    /// World normals of the vertices (computed on first request).
    pub fn normals(&self) -> &[[f32; 3]] {
        &self.vertices_list().1
    }

    /// Number of vertices, without computing them.
    pub fn len(&self) -> usize {
        self.len
    }

    fn vertices_list(&self) -> &(Vec<[f32; 3]>, Vec<[f32; 3]>) {
        self.vertices_list.get_or_init(|| {
            let (mut positions, mut normals) = (Vec::new(), Vec::new());
            if let Some(f) = &self.deferred {
                f(&mut positions, &mut normals);
            }
            (positions, normals)
        })
    }
}

impl WorldGeometry {
    /// Geometry of `model` placed at `placement`: the same as `new` on `PropModel::vertices`.
    /// Only the box is computed right away (cartoon and light selection only read
    /// it); positions and normals are computed if vertex lighting is redone.
    /// `model` must return the PropModel kept by `kept_value`.
    pub fn of_prop<G: 'static>(kept: G, model: fn(&G) -> &openxiii_assets::character::PropModel, placement: &MeshActor) -> Self {
        // Only Location, Rotation, DrawScale3D and PrePivot are used for placement.
        let place = MeshActor {
            actor: String::new(),
            mesh: String::new(),
            location: placement.location,
            rotation: placement.rotation,
            scale: placement.scale,
            pre_pivot: placement.pre_pivot,
            blocks: placement.blocks,
        };
        let m = model(&kept);
        if !kept_active() {
            // Full computation right away (old path).
            let (mut positions, mut normals) = (Vec::new(), Vec::new());
            m.positions_normals(&place, &mut positions, &mut normals);
            let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
            for position in &positions {
                for k in 0..3 {
                    min[k] = min[k].min(position[k]);
                    max[k] = max[k].max(position[k]);
                }
            }
            return Self { min, max, len: positions.len(), vertices_list: std::cell::OnceCell::from((positions, normals)), deferred: None };
        }
        let (min, max) = m.bounds_list(&place);
        let len = m.vertex_count();
        let deferred: Deferred = Box::new(move |positions, normals| model(&kept).positions_normals(&place, positions, normals));
        Self { min, max, len, vertices_list: std::cell::OnceCell::new(), deferred: Some(deferred) }
    }
}

pub fn placement_bits(p: &MeshActor) -> [u32; 12] {
    let l = p.location.map(f32::to_bits);
    let r = p.rotation.map(|x| x as u32);
    let s = p.scale.map(f32::to_bits);
    let t = p.pre_pivot.map(f32::to_bits);
    [l[0], l[1], l[2], r[0], r[1], r[2], s[0], s[1], s[2], t[0], t[1], t[2]]
}

#[derive(PartialEq, Eq)]
pub enum Pose {
    Refresh { matrix: [u32; 16], visible: bool },
    Placement([u32; 12]),
}

#[derive(Default)]
pub struct GeometrySlot(Option<(Pose, Rc<WorldGeometry>)>);
impl GeometrySlot {
    pub fn get(&mut self, key: Pose, build: impl FnOnce() -> WorldGeometry) -> Rc<WorldGeometry> {
        if let Some((old, geometry)) = &self.0 {
            if *old == key {
                return Rc::clone(geometry);
            }
        }
        let geometry = Rc::new(build());
        self.0 = Some((key, Rc::clone(&geometry)));
        geometry
    }
}

#[derive(Default)]
pub struct ActorGeometry {
    pub current: GeometrySlot,
    pub first_render: GeometrySlot,
}

