//! Resident projector fragments, kept only as long as the view uses them.
use super::{SceneVertex, target_clip};
use bytemuck::Zeroable;
use std::{ops::Range, rc::{Rc, Weak}};
use openxiii_vm::projector::ProjectorDraw;

pub(super) type Id = (usize, [usize; 3]);

pub(super) struct Fragment {
    life: Weak<()>,
    source: usize,
    seen: u64,
    shadow: bool,
    pub poly: Rc<[[f32; 3]]>,
    triangles: bool,
    pub vertices: Range<usize>,
    pub indices: Rc<[u32]>,
    slot: Rc<()>,
    pub bounds: Option<target_clip::Bounds>,
    pub stages: Option<(u64, Option<(bool, bool)>)>,
    matrix: Option<[u32; 16]>,
}

// Shadows have no per-vertex matrix. Two live generations can
// therefore share the same immutable vertices; any change detaches the range.
struct ShadowGeometry {
    slot: Weak<()>,
    poly: Rc<[[f32; 3]]>,
    triangles: bool,
    vertices: Range<usize>,
    indices: Rc<[u32]>,
    bounds: Option<target_clip::Bounds>,
}

#[derive(Default)]
pub(super) struct Arena {
    pub fragments: rustc_hash::FxHashMap<Id, Fragment>,
    // One lifetime check per impact, rather than one per BSP fragment.
    lives: rustc_hash::FxHashMap<usize, (Weak<()>, Vec<Id>)>,
    pub vertices: Vec<SceneVertex>,
    pub matrices: Vec<[f32; 16]>,
    free: Vec<Range<usize>>,
    free_dirty: bool,
    shadows: rustc_hash::FxHashMap<(usize, [usize; 3]), ShadowGeometry>,
    retired: rustc_hash::FxHashMap<(usize, [usize; 3]), Fragment>,
    pub dirty: Vec<Range<usize>>,
    pub matrix_dirty: Vec<Range<usize>>,
    pub visible: Vec<Id>,
    pub indices: Vec<u32>,
    pub rebuilt: usize,
    view: u64,
}

impl Arena {
    pub fn begin(&mut self) {
        self.view = self.view.wrapping_add(1);
        self.dirty.clear();
        self.matrix_dirty.clear();
        self.visible.clear();
        self.indices.clear();
        self.rebuilt = 0;
        // Shadows reattach a new info every frame. A dead fragment can
        // give its slot to its successor from the same projector/receiver, if its
        // bytes are identical. Two generations still alive are never confused.
        for (_, f) in self.retired.drain() {
            if Rc::strong_count(&f.slot) == 1 { self.free.push(f.vertices); self.free_dirty = true; }
        }
        let dead: Vec<_> = self.lives.iter().filter(|(_, (life, _))| life.strong_count() == 0).map(|(&id, _)| id).collect();
        for life in dead {
            let (_, ids) = self.lives.remove(&life).unwrap();
            for id in ids {
                let f = self.fragments.remove(&id).unwrap();
                if let Some(old) = self.retired.insert((f.source, id.1), f) {
                    if Rc::strong_count(&old.slot) == 1 { self.free.push(old.vertices); self.free_dirty = true; }
                }
            }
        }
        self.coalesce_free();
    }

    fn coalesce_free(&mut self) {
        if !self.free_dirty { return; }
        self.free_dirty = false;
        self.free.sort_unstable_by_key(|r| r.start);
        let mut n = 0;
        for i in 0..self.free.len() {
            let r = self.free[i].clone();
            if n > 0 && self.free[n - 1].end == r.start { self.free[n - 1].end = r.end; }
            else { self.free[n] = r; n += 1; }
        }
        self.free.truncate(n);
        // Only truncate a free tail; no live range is moved.
        while self.free.last().is_some_and(|r| r.end == self.vertices.len()) {
            let r = self.free.pop().unwrap();
            self.vertices.truncate(r.start);
            self.matrices.truncate(r.start);
        }
    }

    pub fn end(&mut self) {
        // The VM may keep an old off-screen shadow for a long time. The render
        // cache only keeps the fragments used by the new view.
        let free = &mut self.free;
        let dirty = &mut self.free_dirty;
        let view = self.view;
        self.fragments.retain(|_, f| {
            if f.seen == view { return true; }
            if Rc::strong_count(&f.slot) == 1 { free.push(f.vertices.clone()); *dirty = true; }
            false
        });
        let fragments = &self.fragments;
        self.lives.retain(|_, (_, ids)| {
            ids.retain(|id| fragments.contains_key(id));
            !ids.is_empty()
        });
        for (_, f) in self.retired.drain() {
            if Rc::strong_count(&f.slot) == 1 { self.free.push(f.vertices); self.free_dirty = true; }
        }
        self.shadows.retain(|_, g| g.slot.strong_count() != 0);
        self.coalesce_free();
        let len = self.vertices.len();
        self.dirty.retain_mut(|r| { r.end = r.end.min(len); r.start < r.end });
    }

    pub fn can_matrix(&self, index: usize) -> bool {
        !self.fragments[&self.visible[index]].shadow
    }

    fn allocate(&mut self, len: usize) -> Range<usize> {
        if let Some(i) = self.free.iter().position(|r| r.len() >= len) {
            let start = self.free[i].start;
            self.free[i].start += len;
            if self.free[i].is_empty() { self.free.remove(i); }
            return start..start + len;
        }
        let start = self.vertices.len();
        self.vertices.resize(start + len, SceneVertex::zeroed());
        start..start + len
    }

    pub fn add(&mut self, d: &ProjectorDraw, negative: bool) -> (u32, u32) {
        self.update(&d.info.geometry_life, d.info.projector, d.fragment, &d.poly, d.triangles, negative, d.texture.starts_with("#shadow:"))
    }

    fn update(&mut self, life: &Rc<()>, source: usize, fragment: [usize; 3], poly: &Rc<[[f32; 3]]>, triangles: bool, negative: bool, shadow: bool) -> (u32, u32) {
        let id = (Rc::as_ptr(life) as usize, fragment);
        if !self.fragments.contains_key(&id) {
            self.lives.entry(id.0).or_insert_with(|| (Rc::downgrade(life), Vec::new())).1.push(id);
            if let Some(mut f) = self.retired.remove(&(source, fragment)) {
                f.life = Rc::downgrade(life);
                self.fragments.insert(id, f);
            } else if shadow {
                if let Some(g) = self.shadows.get(&(source, fragment)).filter(|g| g.triangles == triangles
                    && (Rc::ptr_eq(&g.poly, poly) || bytemuck::cast_slice::<_, u8>(&g.poly) == bytemuck::cast_slice::<_, u8>(poly))) {
                    if let Some(slot) = g.slot.upgrade() {
                        self.fragments.insert(id, Fragment {
                            life: Rc::downgrade(life), source, seen: self.view, shadow, poly: poly.clone(), triangles,
                            vertices: g.vertices.clone(), indices: g.indices.clone(), slot,
                            bounds: g.bounds, stages: None, matrix: None,
                        });
                    }
                }
            }
        }
        let same = self.fragments.get(&id).is_some_and(|f| f.shadow == shadow && f.triangles == triangles
            && (Rc::ptr_eq(&f.poly, &poly) || bytemuck::cast_slice::<_, u8>(&f.poly) == bytemuck::cast_slice::<_, u8>(&poly)));
        if !same {
            let previous = self.fragments.remove(&id);
            let range = match previous {
                Some(f) if Rc::strong_count(&f.slot) == 1 && f.vertices.len() == poly.len() => f.vertices,
                Some(f) => {
                    if Rc::strong_count(&f.slot) == 1 { self.free.push(f.vertices); self.free_dirty = true; }
                    self.allocate(poly.len())
                }
                None => self.allocate(poly.len()),
            };
            for (dst, p) in self.vertices[range.clone()].iter_mut().zip(poly.iter()) {
                *dst = SceneVertex { position: *p, normal: [0.; 3], color: [255; 4], uv0: [0.; 2], uv1: [0.; 2] };
            }
            // Test only: first vertex shifted on purpose, the geometry and draw test must notice it.
            if negative { self.vertices[range.start].position[0] += 1.; }
            let base = range.start as u32;
            let indices: Rc<[u32]> = if triangles { (base..base + (poly.len() / 3 * 3) as u32).collect() }
                else { (1..poly.len() as u32 - 1).flat_map(|k| [base, base + k, base + k + 1]).collect() };
            self.rebuilt += range.len();
            self.dirty.push(range.clone());
            self.fragments.insert(id, Fragment {
                life: Rc::downgrade(life), source, seen: self.view, shadow, poly: poly.clone(), triangles,
                vertices: range, indices, slot: Rc::new(()), bounds: target_clip::Bounds::from_world(poly.iter().copied()),
                stages: None, matrix: None,
            });
        }
        // Also keep the new Rc when its bytes are unchanged: the producer
        // can then reuse this Rc, without comparing the whole polygon again.
        let f = self.fragments.get_mut(&id).unwrap();
        f.seen = self.view;
        f.shadow = shadow;
        if !Rc::ptr_eq(&f.poly, poly) { f.poly = poly.clone(); }
        if shadow {
            self.shadows.insert((source, fragment), ShadowGeometry {
                slot: Rc::downgrade(&f.slot), poly: f.poly.clone(), triangles,
                vertices: f.vertices.clone(), indices: f.indices.clone(), bounds: f.bounds,
            });
        }
        let first = self.indices.len() as u32;
        self.indices.extend_from_slice(&f.indices);
        self.visible.push(id);
        (first, f.indices.len() as u32)
    }

    fn resident_ranges(&self, matrices: bool) -> Vec<Range<usize>> {
        let mut ranges: Vec<_> = self.fragments.values().chain(self.retired.values())
            .filter(|f| !matrices || f.matrix.is_some()).map(|f| f.vertices.clone()).collect();
        merge_ranges(&mut ranges);
        ranges
    }

    pub fn matrix(&mut self, index: usize, matrix: [f32; 16]) {
        let f = self.fragments.get_mut(&self.visible[index]).unwrap();
        let bits = matrix.map(f32::to_bits);
        if f.matrix == Some(bits) { return; }
        f.matrix = Some(bits);
        // Do not initialise 64 extra bytes for each shadow vertex.
        if self.matrices.len() < f.vertices.end { self.matrices.resize(f.vertices.end, [0.; 16]); }
        self.matrices[f.vertices.clone()].fill(matrix);
        self.matrix_dirty.push(f.vertices.clone());
    }
}

/// Largest hole (in vertices, 64 bytes each) covered to merge two matrix
/// uploads: 16 KiB more at worst per upload removed.
const HOLE_MATRICES_MAX: usize = 256;

/// Sorted, disjoint ranges (`merge_ranges`): two ranges separated by at most `hole_max`
/// vertices become one, which also uploads the hole. The hole's matrices are those of the
/// mirror: the same bytes as the buffer for fragments already uploaded, and for vertices
/// without a fragment bytes no draw reads. Gun recoil (the camera moves, all
/// impact matrices change): ~26 fewer small uploads per frame.
pub(super) fn small_merge_holes(ranges: &mut Vec<Range<usize>>, hole_max: usize) {
    let mut n = 0;
    for i in 0..ranges.len() {
        let r = ranges[i].clone();
        if n > 0 && r.start - ranges[n - 1].end <= hole_max { ranges[n - 1].end = r.end; }
        else { ranges[n] = r; n += 1; }
    }
    ranges.truncate(n);
}

#[cfg(test)]
#[test]
fn small_merge_holes_ranges() {
    let mut r = vec![0..4, 10..12, 200..210, 467..470, 900..901];
    small_merge_holes(&mut r, 256);
    assert_eq!(r, vec![0..210, 467..470, 900..901]);
    let mut r = vec![0..4, 10..12];
    small_merge_holes(&mut r, 5);
    assert_eq!(r, vec![0..4, 10..12]);
    small_merge_holes(&mut r, 6);
    assert_eq!(r, vec![0..12]);
    let mut empty: Vec<Range<usize>> = Vec::new();
    small_merge_holes(&mut empty, 256);
    assert!(empty.is_empty());
}

/// Only merge neighbouring writes, without including unchanged fragments.
pub(super) fn merge_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_unstable_by_key(|r| r.start);
    let mut n = 0;
    for i in 0..ranges.len() {
        let r = ranges[i].clone();
        if n > 0 && r.start <= ranges[n - 1].end { ranges[n - 1].end = ranges[n - 1].end.max(r.end); }
        else { ranges[n] = r; n += 1; }
    }
    ranges.truncate(n);
}

use super::{Renderer, Geometry, Stage, DrawCmd, PASS_STRIDE, write_value};

fn buffer(device: &wgpu::Device, queue: &wgpu::Queue, old: Option<wgpu::Buffer>, size: usize, usage: wgpu::BufferUsages) -> (wgpu::Buffer, bool, usize) {
    if old.as_ref().is_some_and(|b| b.size() >= size as u64) { return (old.unwrap(), false, 0); }
    let b = device.create_buffer(&wgpu::BufferDescriptor {
        // First size large enough for any fight seen over every map (1 MB of vertices, 128 KB of
        // indices at most), so that the buffers do not grow again while playing.
        label: Some("impact fragments"), size: size.max(if usage.contains(wgpu::BufferUsages::INDEX) { 256 << 10 } else { 2 << 20 }).next_power_of_two() as u64,
        usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
    });
    let growth = b.size() as usize;
    if let Some(old) = old {
        if !super::empty_window() {
            let mut encoder = super::take_encoder(device, queue);
            encoder.copy_buffer_to_buffer(&old, 0, &b, 0, Some(old.size()));
            // The copy and later writes must stay after the views
            // already encoded. Otherwise a direct write_buffer would be overwritten by the copy.
            super::note_read_back(queue, &old);
            super::note_read_back(queue, &b);
            super::return_encoder(queue, encoder);
        }
        (b, false, growth)
    } else { (b, true, growth) }
}

impl Renderer {
    pub(super) fn set_bsp_projectors_incremental(&mut self, draws: &[ProjectorDraw], library: &mut openxiii_package::library::Library) {
        self.impact_arena.begin();
        self.projector_polys.clear();
        self.projector_bounds.clear();
        let broken = false;
        let (old, mut batches_list) = self.projectors.take().map_or((None, Vec::new()), |(g, l)| (Some(g), l));
        batches_list.clear();
        for (i, d) in draws.iter().enumerate() {
            // Evaluation, including its effects, stays in the original order.
            let Some(key) = self.materials.projector_key_at(i, library, &d.texture, d.info.flags, &d.info.matrix, &d.info.gradient) else { continue };
            if d.poly.len() < 3 { continue; }
            let (first, count) = self.impact_arena.add(d, broken);
            self.projector_polys.push((d.poly.clone(), d.triangles));
            let info = self.projector_rect_info(d, library);
            self.projector_rect_infos.truncate(self.projector_polys.len() - 1);
            self.projector_rect_infos.push(info);
            self.projector_bounds.push(self.impact_arena.fragments[self.impact_arena.visible.last().unwrap()].bounds);
            batches_list.push((key, first, count, if d.triangles { 0 } else { 8 }, (!d.triangles).then_some(d.node as u32)));
        }
        self.impact_arena.end();
        self.projector_packets.truncate(batches_list.len());
        let (old_v, old_i) = old.map(|g| (g.vertex_buffer, g.index_buffer)).unzip();
        let (v, new_v, _growth) = buffer(&self.device, &self.queue, old_v, self.impact_arena.vertices.len() * std::mem::size_of::<SceneVertex>(), wgpu::BufferUsages::VERTEX);
        if new_v {
            // Holes will only be used after a write of a new fragment.
            for r in self.impact_arena.resident_ranges(false) {
                let offset = (r.start * std::mem::size_of::<SceneVertex>()) as u64;
                let data = bytemuck::cast_slice(&self.impact_arena.vertices[r]);
                write_value(&self.queue, &v, offset, data);
            }
        } else {
            merge_ranges(&mut self.impact_arena.dirty);
            for r in &self.impact_arena.dirty {
                let data = bytemuck::cast_slice(&self.impact_arena.vertices[r.clone()]);
                write_value(&self.queue, &v, (r.start * std::mem::size_of::<SceneVertex>()) as u64, data);
            }
        }
        // Only the ordered list of references varies when visibility changes.
        // The triangles of each fragment are neither recomputed nor moved.
        let indices = &self.impact_arena.indices;
        let (i, new_i, _growth) = buffer(&self.device, &self.queue, old_i, indices.len() * 4, wgpu::BufferUsages::INDEX);
        // Slice equality uses a block comparison. Element-by-element
        // traversal and the CPU copy are kept for lists that actually changed.
        if new_i || *indices != self.impact_indices_gpu {
        let mut first = 0;
        while first < indices.len() {
            if !new_i && self.impact_indices_gpu.get(first) == Some(&indices[first]) { first += 1; continue; }
            let start = first;
            first += 1;
            while first < indices.len() && (new_i || self.impact_indices_gpu.get(first) != Some(&indices[first])) { first += 1; }
            let data = bytemuck::cast_slice(&indices[start..first]);
            write_value(&self.queue, &i, (start * 4) as u64, data);
        }
        self.impact_indices_gpu.clone_from(indices);
        }
        self.projectors = Some((Geometry { vertex_buffer: v, index_buffer: i, batches: Vec::new() }, batches_list));
    }

    pub(super) fn upload_impact_matrices(&mut self) {
        if !super::matrices_impacts() { return; }
        let (b, new, _growth) = buffer(&self.device, &self.queue, self.impact_matrices_gpu.take(), self.impact_arena.matrices.len() * 64, wgpu::BufferUsages::VERTEX);
        if new {
            for r in self.impact_arena.resident_ranges(true) {
                let offset = (r.start * 64) as u64;
                let data = bytemuck::cast_slice(&self.impact_arena.matrices[r]);
                write_value(&self.queue, &b, offset, data);
            }
        } else {
            merge_ranges(&mut self.impact_arena.matrix_dirty);
            small_merge_holes(&mut self.impact_arena.matrix_dirty, HOLE_MATRICES_MAX);
            for r in &self.impact_arena.matrix_dirty {
                let data = bytemuck::cast_slice(&self.impact_arena.matrices[r.clone()]);
                write_value(&self.queue, &b, (r.start * 64) as u64, data);
            }
        }
        self.impact_arena.matrix_dirty.clear();
        self.impact_matrices_gpu = Some(b);
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn polygon(i: usize, frame: usize) -> Rc<[[f32; 3]]> {
        let x = i as f32 * 3.;
        vec![[x, -0., 1.], [x + 1., 0., 2.], [x + 1., 1., 3.], [x, 1., frame as f32]].into()
    }

    #[test]
    fn impacts_160_views_spawns_despawns_and_control() {
        let mut arena = Arena::default();
        let mut negative = Arena::default();
        let mut objects: Vec<(Rc<()>, Rc<[[f32; 3]]>)> = Vec::new();
        let mut rejected = 0;
        let mut kept = 0;
        for frame in 0..160 {
            if frame % 3 == 0 { objects.push((Rc::new(()), polygon(frame, frame))); }
            if frame % 7 == 0 && !objects.is_empty() { objects.remove(0); }
            if frame % 11 == 0 && !objects.is_empty() { objects[0].1 = polygon(999, frame); }
            arena.begin(); negative.begin();
            // Two orientations and two views: absences and returns without destruction.
            let mut visible: Vec<_> = (0..objects.len()).filter(|i| (i + frame) % 4 != 0).collect();
            if frame % 2 == 0 { visible.reverse(); }
            let mut expected = Vec::<SceneVertex>::new();
            for i in visible {
                let (life, poly) = &objects[i];
                let id = [0, i % 5, 0];
                let before = arena.fragments.get(&(Rc::as_ptr(life) as usize, id)).map(|f| f.vertices.clone());
                let (first, count) = arena.update(life, i, id, poly, false, false, false);
                negative.update(life, i, id, poly, false, true, false);
                if let Some(range) = before { assert_eq!(range, arena.fragments[arena.visible.last().unwrap()].vertices); kept += 1; }
                assert_eq!(count, 6);
                assert_eq!(first as usize + count as usize, arena.indices.len());
                for k in [0, 1, 2, 0, 2, 3] {
                    expected.push(SceneVertex { position: poly[k], normal: [0.; 3], color: [255; 4], uv0: [0.; 2], uv1: [0.; 2] });
                }
            }
            let actual: Vec<_> = arena.indices.iter().map(|&i| arena.vertices[i as usize]).collect();
            assert_eq!(bytemuck::cast_slice::<_, u8>(&actual), bytemuck::cast_slice::<_, u8>(&expected));
            let bad: Vec<_> = negative.indices.iter().map(|&i| negative.vertices[i as usize]).collect();
            rejected += (bytemuck::cast_slice::<_, u8>(&bad) != bytemuck::cast_slice::<_, u8>(&expected)) as usize;
            arena.end(); negative.end();
            assert!(arena.fragments.len() <= arena.visible.len());
        }
        assert!(kept > 100 && rejected > 100, "{kept} kept; {rejected} errors detected");
        objects.clear(); arena.begin();
        assert!(arena.fragments.is_empty(), "dead impacts must release their fragments");
        println!("160 exact views; {kept} kept; {rejected} altered copies rejected");
    }

    #[test]
    fn impacts_bits_matrix_sign_zero_and_reuse() {
        let mut arena = Arena::default();
        let life = Rc::new(());
        let p = polygon(0, 0);
        arena.begin(); arena.update(&life, 0, [0, 1, 0], &p, false, false, false);
        let range = arena.fragments[&arena.visible[0]].vertices.clone();
        let mut m = [0.; 16]; m[1] = -0.;
        arena.matrix(0, m);
        assert_eq!(arena.matrices[range.start][1].to_bits(), (-0f32).to_bits());
        arena.begin(); arena.update(&life, 0, [0, 1, 0], &p.clone(), false, false, false);
        arena.matrix(0, m);
        assert_eq!(arena.rebuilt, 0);
        assert!(arena.dirty.is_empty() && arena.matrix_dirty.is_empty());
        m[1] = 0.; arena.matrix(0, m);
        assert_eq!(arena.matrix_dirty, vec![range]);
    }

    #[test]
    fn impacts_reattach_shadow_without_rebuild_distinct_generations() {
        let mut arena = Arena::default();
        let mut life = Rc::new(());
        let p = polygon(1, 0);
        arena.begin(); arena.update(&life, 7, [0, 2, 0], &p, false, false, false);
        let range = arena.fragments[&arena.visible[0]].vertices.clone();
        life = Rc::new(()); // The old info has disappeared; same receiver, same bytes.
        arena.begin(); arena.update(&life, 7, [0, 2, 0], &p, false, false, false);
        assert_eq!(arena.rebuilt, 0);
        assert_eq!(arena.fragments[&arena.visible[0]].vertices, range);
        let second = Rc::new(()); // Two live generations: do not share the matrix stream.
        arena.update(&second, 7, [0, 2, 0], &p, false, false, false);
        assert_ne!(arena.fragments[&arena.visible[1]].vertices, range);
    }
    #[test]
    fn shared_shadows_detach_and_release() {
        let mut arena = Arena::default();
        let a = Rc::new(());
        let b = Rc::new(());
        let poly = polygon(1, 0);
        arena.begin();
        arena.update(&a, 7, [0, 2, 0], &poly, false, false, true);
        let old = arena.fragments[&arena.visible[0]].vertices.clone();
        let original = bytemuck::cast_slice::<_, u8>(&arena.vertices[old.clone()]).to_vec();
        arena.begin();
        arena.update(&b, 7, [0, 2, 0], &poly, false, false, true);
        assert_eq!(arena.fragments[&arena.visible[0]].vertices, old);
        assert_eq!(arena.rebuilt, 0);
        assert!(arena.matrices.is_empty(), "no matrix stream for the shadows");
        // Only one generation deforms; the one still alive keeps its bytes.
        let changed = polygon(1, 9);
        arena.begin();
        arena.update(&b, 7, [0, 2, 0], &changed, false, false, true);
        let new = arena.fragments[&arena.visible[0]].vertices.clone();
        assert_ne!(new, old);
        assert_eq!(bytemuck::cast_slice::<_, u8>(&arena.vertices[old.clone()]), original);
        drop(a);
        arena.begin(); arena.begin(); // Removal then recycling of the dead generation.
        assert!(arena.free.iter().any(|r| r.start <= old.start && r.end >= old.end));
        assert!(!arena.free.iter().any(|r| r.start < new.end && new.start < r.end));
        let live = arena.resident_ranges(false);
        assert_eq!(live, vec![new]);
        assert!(arena.resident_ranges(true).is_empty());
    }

    #[test]
    fn alive_shadow_outside_view_evicted_and_return_exact() {
        let mut arena = Arena::default();
        let mut lives = Vec::new();
        for frame in 0..160 {
            let life = Rc::new(());
            let poly = polygon(0, frame);
            arena.begin();
            arena.update(&life, 7, [0, 2, 0], &poly, false, false, true);
            arena.end();
            lives.push(life); // The VM deliberately keeps every generation.
            assert_eq!(arena.fragments.len(), 1);
            assert_eq!(arena.lives.len(), 1);
            assert!(arena.retired.is_empty());
            assert!(arena.vertices.len() <= 8, "only two ranges of four vertices are needed");
            for (i, p) in poly.iter().enumerate() {
                let r = arena.fragments[&arena.visible[0]].vertices.clone();
                assert_eq!(arena.vertices[r.start+i].position.map(f32::to_bits), p.map(f32::to_bits));
            }
        }
        // An old view comes back: the old generation is rebuilt exactly.
        let poly = polygon(0, 0);
        arena.begin();
        arena.update(&lives[0], 7, [0, 2, 0], &poly, false, false, true);
        arena.end();
        assert_eq!(arena.fragments.len(), 1);
        let r = arena.fragments[&arena.visible[0]].vertices.clone();
        assert_eq!(arena.vertices[r.end-1].position.map(f32::to_bits), poly[3].map(f32::to_bits));
        arena.begin(); arena.end(); // No geometry in this view.
        assert!(arena.fragments.is_empty() && arena.lives.is_empty() && arena.shadows.is_empty());
        assert!(arena.vertices.is_empty() && arena.matrices.is_empty());
    }

    #[test]
    fn shared_shadow_reclassified_detached_before_its_matrix() {
        let mut arena = Arena::default();
        let a = Rc::new(()); let b = Rc::new(());
        let poly = polygon(1, 0);
        arena.begin();
        arena.update(&a, 7, [0, 2, 0], &poly, false, false, true);
        arena.update(&b, 7, [0, 2, 0], &poly, false, false, true);
        arena.end();
        assert_eq!(arena.fragments[&arena.visible[0]].vertices, arena.fragments[&arena.visible[1]].vertices);
        arena.begin();
        arena.update(&a, 7, [0, 2, 0], &poly, false, false, false);
        arena.update(&b, 7, [0, 2, 0], &poly, false, false, true);
        arena.end();
        assert!(arena.can_matrix(0)); assert!(!arena.can_matrix(1));
        assert_ne!(arena.fragments[&arena.visible[0]].vertices, arena.fragments[&arena.visible[1]].vertices);
        arena.matrix(0, [3.5; 16]);
        assert_eq!(arena.matrix_dirty, vec![arena.fragments[&arena.visible[0]].vertices.clone()]);
    }

}

