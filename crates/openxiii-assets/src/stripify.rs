//! Strip-based reordering of a section's triangles: FRawIndexBuffer vtable +0x10
//! calls the stripifier (Opcode strips: adjacencies,
//! strip following, strip choice, joining), then
//! cuts the single strip back into a triangle list (degenerate triangles are skipped).

/// Adjacencies: per face, neighbour and neighbour's edge for each edge, plus the
/// mark array (also the mask of linked edges during construction).
struct Adj<'a> {
    n: usize,
    idx: &'a [u16],
    nb: Vec<[i32; 3]>,
    ed: Vec<[i32; 3]>,
    marks: Vec<i32>,
}

impl<'a> Adj<'a> {
    fn new(n: usize, idx: &'a [u16]) -> Self {
        let mut a = Adj { n, idx, nb: vec![[-1; 3]; n], ed: vec![[0; 3]; n], marks: vec![0; n] };
        for f in 0..n {
            for e in 0..3 {
                if a.marks[f] & (1 << e) == 0 {
                    a.link(f, e);
                }
            }
        }
        a.marks.iter_mut().for_each(|m| *m = 0);
        a
    }

    fn v(&self, f: usize, k: usize) -> u16 {
        self.idx[f * 3 + k]
    }

    /// First following incomplete face that has an inverse edge.
    fn link(&mut self, f: usize, e: usize) {
        let a = self.v(f, (e + 1) % 3);
        let b = self.v(f, e);
        for g in f + 1..self.n {
            if self.marks[g] == 7 {
                continue;
            }
            for k in 0..3 {
                if self.v(g, k) == a && self.v(g, (k + 1) % 3) == b {
                    self.nb[f][e] = g as i32;
                    self.ed[f][e] = k as i32;
                    self.marks[f] |= 1 << e;
                    self.nb[g][k] = f as i32;
                    self.ed[g][k] = e as i32;
                    self.marks[g] |= 1 << k;
                    return;
                }
            }
        }
    }

    fn free(&self, f: i32) -> i32 {
        self.nb[f as usize].iter().filter(|&&n| n != -1 && self.marks[n as usize] == 0).count() as i32
    }

    fn open(&self, f: i32) -> bool {
        f != -1 && self.marks[f as usize] == 0
    }

    /// Follows a strip from face `face`, edge `edge`. Returns its length; `dup`
    /// receives the number of joining vertices added.
    #[allow(clippy::too_many_arguments)]
    fn track(&mut self, face: i32, edge: i32, limit: usize, dup: &mut i32, look: bool, flip: bool, strip: &mut [i32], edges: &mut [i32]) -> usize {
        *dup = 0;
        if self.marks[face as usize] != 0 {
            return 0;
        }
        self.marks[face as usize] = 1;
        let mut count = 0;
        let (mut f, mut e, mut fl) = (face, edge, flip);
        strip[0] = f;
        strip[1] = f;
        strip[2] = f;
        if fl {
            edges[0] = e % 3;
            edges[1] = (e + 1) % 3;
        } else {
            edges[0] = (e + 1) % 3;
            edges[1] = e % 3;
        }
        edges[2] = (e + 2) % 3;
        fl = !fl;
        let s = ((if fl { e + 2 } else { e + 1 }) % 3) as usize;
        let mut nface = self.nb[f as usize][s];
        let mut nedge = self.ed[f as usize][s];
        let mut len = 3;
        while len < limit {
            if !self.open(nface) {
                break;
            }
            f = nface;
            e = nedge;
            fl = !fl;
            let e2 = ((if fl { e + 2 } else { e + 1 }) % 3) as usize;
            nface = self.nb[f as usize][e2];
            nedge = self.ed[f as usize][e2];
            let mut stop = false;
            if !self.open(nface) {
                stop = true;
            } else if look {
                let e3 = ((if fl { e + 1 } else { e + 2 }) % 3) as usize;
                let c = self.nb[f as usize][e3];
                let ced = self.ed[f as usize][e3];
                if self.open(c) {
                    let (cc, cn) = (self.free(c), self.free(nface));
                    if cc < cn {
                        stop = true;
                    } else if cc == cn {
                        let t = ((if fl { ced + 2 } else { ced + 1 }) % 3) as usize;
                        let c2 = self.nb[c as usize][t];
                        let u = ((if fl { nedge + 1 } else { nedge + 2 }) % 3) as usize;
                        let x = self.nb[nface as usize][u];
                        if !self.open(x) {
                            stop = true;
                        } else if self.open(c2) && self.free(c2) < self.free(x) {
                            stop = true;
                        }
                    }
                }
            }
            if stop {
                let e4 = ((if fl { e + 1 } else { e + 2 }) % 3) as usize;
                nface = self.nb[f as usize][e4];
                nedge = self.ed[f as usize][e4];
                if self.open(nface) {
                    strip[len] = strip[len - 2];
                    edges[len] = edges[len - 2];
                    len += 1;
                    count += 1;
                    fl = !fl;
                }
            }
            strip[len] = f;
            edges[len] = (e + 2) % 3;
            self.marks[f as usize] = 1;
            len += 1;
        }
        for i in 2..len {
            self.marks[strip[i] as usize] = 0;
        }
        *dup = count;
        len
    }
}

struct Strip {
    idx: Vec<u16>,
    flag: bool,
}

/// Successive strips, each time the best one over all free faces (fewest
/// free neighbours first, then the smallest joining-vertices / length ratio).
fn compute(adj: &mut Adj, limit: usize, look: bool) -> Vec<Strip> {
    adj.marks.iter_mut().for_each(|m| *m = 0);
    let mut flip = true;
    let mut strips = Vec::new();
    let mut strip = vec![0i32; 0x400 + 4];
    let mut edges = vec![0i32; 0x400 + 4];
    loop {
        let (mut best_face, mut best_edge, mut best_score, mut best_n) = (0usize, 0usize, 2.0f32, i32::MAX);
        for f in 0..adj.n {
            if adj.marks[f] != 0 {
                continue;
            }
            let mut c = adj.free(f as i32);
            if c == 0 {
                c = 4;
            }
            if c > best_n {
                continue;
            }
            for e in 0..3 {
                let mut dup = 0;
                let len = adj.track(f as i32, e as i32, limit, &mut dup, look, flip, &mut strip, &mut edges);
                let score = if len == 3 { 1.0f32 } else { dup as f32 / len as f32 };
                if c < best_n || (c == best_n && score < best_score) {
                    best_n = c;
                    best_face = f;
                    best_edge = e;
                    best_score = score;
                }
            }
        }
        if best_n == i32::MAX {
            break;
        }
        let mut dup = 0;
        let len = adj.track(best_face as i32, best_edge as i32, limit, &mut dup, look, flip, &mut strip, &mut edges);
        for &s in &strip[..len] {
            adj.marks[s as usize] = 1;
        }
        let idx = (0..len).map(|i| adj.idx[strip[i] as usize * 3 + edges[i] as usize]).collect();
        strips.push(Strip { idx, flag: flip });
        if len & 1 != 0 {
            flip = !flip;
        }
    }
    strips
}

/// 16-vertex cache: hits, vertices, tags, insertion position.
#[derive(Clone)]
struct Cache {
    hits: i32,
    verts: [u16; 16],
    tag: [i32; 16],
    ring: usize,
}

impl Cache {
    fn new() -> Self {
        Cache { hits: 0, verts: [0xFFFF; 16], tag: [0; 16], ring: 0 }
    }

    fn add(&mut self, tag: i32, v: u16) {
        for i in 0..16 {
            if self.verts[i] == v {
                if self.tag[i] != tag {
                    self.hits += 1;
                    self.tag[i] = tag;
                }
                return;
            }
        }
        self.verts[self.ring] = v;
        self.tag[self.ring] = tag;
        self.ring = (self.ring + 1) % 16;
    }
}

/// The strip that gains the most cache hits, moved to the front (reversed if that
/// gains more).
fn pick_best(strips: &mut [Strip], cache: &Cache) {
    let first_flag = strips[0].flag;
    let mut best = 0usize;
    let mut best_len = strips[0].idx.len();
    let mut reversed = false;
    let mut best_score = -1;
    for i in 0..strips.len() {
        let len = strips[i].idx.len();
        if strips[i].flag != first_flag || (best_len & 1) != (len & 1) {
            continue;
        }
        let mut c1 = cache.clone();
        for &v in &strips[i].idx {
            c1.add(2, v);
        }
        let mut rev = false;
        if len & 1 == 0 {
            let mut c2 = cache.clone();
            for &v in strips[i].idx.iter().rev() {
                c2.add(2, v);
            }
            if c2.hits > c1.hits {
                c1 = c2;
                rev = true;
            }
        }
        let score = c1.hits - cache.hits;
        if score > best_score {
            best_score = score;
            best = i;
            best_len = len;
            reversed = rev;
        }
    }
    if reversed {
        strips[best].idx.reverse();
    }
    strips.swap(0, best);
}

/// One single list, the strips linked by degenerate vertices.
fn join(mut strips: Vec<Strip>) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    let mut cache = Cache::new();
    let s0 = strips.remove(0);
    for &v in &s0.idx {
        out.push(v);
        cache.add(1, v);
    }
    while !strips.is_empty() {
        pick_best(&mut strips, &cache);
        let s = strips.remove(0);
        let last = out[out.len() - 1];
        let first = s.idx[0];
        if first != last {
            out.push(last);
            out.push(first);
        }
        if s.flag != (out.len() & 1 == 0) {
            out.push(first);
        }
        for &v in &s.idx {
            out.push(v);
            cache.add(1, v);
        }
    }
    out
}

/// The strip list (tried with and without look-ahead, the shorter wins).
fn stripify(indices: &[u16]) -> Vec<u16> {
    let n = indices.len() / 3;
    if n == 0 {
        return Vec::new();
    }
    let mut adj = Adj::new(n, indices);
    let mut best: Option<(Vec<Strip>, usize)> = None;
    for look in [true, false] {
        let strips = compute(&mut adj, 0x400, look);
        let cost = strips.iter().map(|s| s.idx.len()).sum::<usize>() + 2 * strips.len() - 2;
        if best.as_ref().is_none_or(|b| cost < b.1) {
            best = Some((strips, cost));
        }
    }
    join(best.unwrap().0)
}

/// FRawIndexBuffer vtable +0x10: triangle list reordered by the strip.
pub fn cache_optimize(indices: &[u16]) -> Vec<u16> {
    let strip = stripify(indices);
    let mut out = Vec::with_capacity(indices.len());
    for b in 0..strip.len().saturating_sub(2) {
        let (x, y, z) = if b & 1 == 1 { (strip[b + 1], strip[b], strip[b + 2]) } else { (strip[b], strip[b + 1], strip[b + 2]) };
        if x != y && y != z && x != z {
            out.extend([x, y, z]);
        }
    }
    out
}

