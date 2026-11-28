//! Screen rectangle of a projector (speed only, identical image). A projector without bClipBSP is drawn over
//! the whole polygon of its BSP node (as in the original): on a large floor, the graphics card
//! processes the whole floor for a small shadow or blood stain (several ms per frame on slow GPUs). Outside
//! the frame of the projected texture (u, v outside [0, 1]) and the frame of ProjClip / ProjGradient, the
//! textures are read at the edge (clamped addressing) and blending changes nothing. So the polygon is clipped
//! by these planes, with a margin, projected to the screen, and drawing is limited to that rectangle
//! (set_scissor_rect): the pixels drawn come from the same triangles as before.

/// Margin in texture coordinates (1/16 of the frame): clipped pixels have u or v clearly outside
/// [0, 1], where the clamped edge of the texture is read.
const MARGIN: f32 = 1.0 / 16.0;

pub fn active_value() -> bool {
    true
}

/// Homogeneous plane a·(x, y, z, 1) ≥ 0.
type Plan = [f32; 4];

/// Column j of the row-major matrix (row vector): c_j = Σ p_i M[i][j].
fn column(m: &[f32; 16], j: usize) -> [f32; 4] {
    [m[j], m[4 + j], m[8 + j], m[12 + j]]
}

fn plans(matrix: &[f32; 16], gradient: &[f32; 16], projected: bool, edge_texture: bool, edge_clip: bool) -> Vec<Plan> {
    let mut v = Vec::with_capacity(9);
    let comb = |a: [f32; 4], ka: f32, b: [f32; 4], kb: f32| -> Plan { std::array::from_fn(|k| ka * a[k] + kb * b[k]) };
    // Projected texture: u = c0 / c2, v = c1 / c2 (D3DTTFF_COUNT3 | PROJECTED), c2 > 0.
    let (c0, c1, c2) = (column(matrix, 0), column(matrix, 1), column(matrix, 2));
    let unit_one = [0.0, 0.0, 0.0, 1.0];
    let w = if projected { c2 } else { unit_one };
    if projected {
        v.push(c2);
    }
    // Texture with wrapped edges: no clipping by its frame.
    for c in [c0, c1].into_iter().filter(|_| edge_texture) {
        v.push(comb(c, 1.0, w, MARGIN)); // u ≥ -margin
        v.push(comb(w, 1.0 + MARGIN, c, -1.0)); // u ≤ 1 + margin
    }
    // ProjClip / ProjGradient: u = g0, v = g1 (not projected).
    let (g0, g1) = (column(gradient, 0), column(gradient, 1));
    for c in [g0, g1].into_iter().filter(|_| edge_clip) {
        v.push(comb(c, 1.0, unit_one, MARGIN));
        v.push(comb(unit_one, 1.0 + MARGIN, c, -1.0));
    }
    v
}

fn split(poly: &mut Vec<[f32; 4]>, plan: &[f32; 4]) {
    let d = |p: &[f32; 4]| plan[0] * p[0] + plan[1] * p[1] + plan[2] * p[2] + plan[3] * p[3];
    let n = poly.len();
    if n == 0 {
        return;
    }
    let mut out = Vec::with_capacity(n + 2);
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let (da, db) = (d(&a), d(&b));
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            out.push(std::array::from_fn(|k| a[k] + t * (b[k] - a[k])));
        }
    }
    *poly = out;
}

/// Rectangle (x, y, width, height) in pixels of target `size_value`, or None when everything must be
/// drawn (polygon touching the near plane, or computation impossible). `view_proj`: view
/// matrix (columns), positions passed in right-handed form (Y flipped) as in the shader.
/// Some([0, 0, 0, 0]): nothing to draw.
pub fn rectangle(poly: &[[f32; 3]], triangles: bool, matrix: &[f32; 16], gradient: &[f32; 16], projected: bool, edge_texture: bool, edge_clip: bool, view_proj: &[[f32; 4]; 4], size_value: [u32; 2]) -> Option<[u32; 4]> {
    if poly.len() < 3 || size_value[0] == 0 || size_value[1] == 0 {
        return None;
    }
    let plans = plans(matrix, gradient, projected, edge_texture, edge_clip);
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    let mut empty = true;
    let mut piece = |pts: &[[f32; 3]]| -> bool {
        let mut p: Vec<[f32; 4]> = pts.iter().map(|q| [q[0], q[1], q[2], 1.0]).collect();
        for pl in &plans {
            split(&mut p, pl);
            if p.is_empty() {
                return true;
            }
        }
        for q in &p {
            let x = [q[0], -q[1], q[2], 1.0];
            let c: [f32; 4] = std::array::from_fn(|r| (0..4).map(|k| view_proj[k][r] * x[k]).sum());
            // Behind or too close to the camera: no safe rectangle.
            if !(c[3] > 1e-3) || !c.iter().all(|v| v.is_finite()) {
                return false;
            }
            let (nx, ny) = (c[0] / c[3], c[1] / c[3]);
            let px = (nx * 0.5 + 0.5) * size_value[0] as f32;
            let py = (0.5 - ny * 0.5) * size_value[1] as f32;
            min = [min[0].min(px), min[1].min(py)];
            max = [max[0].max(px), max[1].max(py)];
            empty = false;
        }
        true
    };
    if triangles {
        for t in poly.chunks_exact(3) {
            if !piece(t) {
                return None;
            }
        }
    } else if !piece(poly) {
        return None;
    }
    if empty {
        return Some([0, 0, 0, 0]);
    }
    // Two extra pixels on each side (rasterisation rounding).
    let x0 = (min[0].floor() - 2.0).clamp(0.0, size_value[0] as f32) as u32;
    let y0 = (min[1].floor() - 2.0).clamp(0.0, size_value[1] as f32) as u32;
    let x1 = (max[0].ceil() + 2.0).clamp(0.0, size_value[0] as f32) as u32;
    let y1 = (max[1].ceil() + 2.0).clamp(0.0, size_value[1] as f32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return Some([0, 0, 0, 0]);
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

/// Screen rectangle of a shadow's frame, without walking its triangles (terrain: thousands):
/// the 8 corners of the u, v (texture) and g1 (ProjClip / ProjGradient) volume within [-margin, 1 + margin],
/// each the solution of a 3×3 system, projected to the screen. None if the volume is degenerate or touches
/// the near plane.
pub fn rectangle_frame(matrix: &[f32; 16], gradient: &[f32; 16], projected: bool, view_proj: &[[f32; 4]; 4], size_value: [u32; 2]) -> Option<[u32; 4]> {
    if size_value[0] == 0 || size_value[1] == 0 {
        return None;
    }
    let (c0, c1, c2) = (column(matrix, 0), column(matrix, 1), column(matrix, 2));
    let g1 = column(gradient, 1);
    let bounds_list = [-MARGIN, 1.0 + MARGIN];
    // Corners in clip coordinates (index: bit 0 = u, bit 1 = v, bit 2 = g).
    let mut corners = [[0f32; 4]; 8];
    for (i, coin) in corners.iter_mut().enumerate() {
        let (u, v, g) = (bounds_list[i & 1], bounds_list[(i >> 1) & 1], bounds_list[(i >> 2) & 1]);
        // Lines a·p = b: u (projected: c0 - u c2 = 0), v, g1 = g.
        let line_value = |c: [f32; 4], k: f32| -> ([f64; 3], f64) {
            if projected {
                ([(c[0] - k * c2[0]) as f64, (c[1] - k * c2[1]) as f64, (c[2] - k * c2[2]) as f64], -((c[3] - k * c2[3]) as f64))
            } else {
                ([c[0] as f64, c[1] as f64, c[2] as f64], (k - c[3]) as f64)
            }
        };
        let (a0, b0) = line_value(c0, u);
        let (a1, b1) = line_value(c1, v);
        let (a2, b2) = ([g1[0] as f64, g1[1] as f64, g1[2] as f64], (g - g1[3]) as f64);
        let det = |m: [[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        let m = [a0, a1, a2];
        let d = det(m);
        let scale_value = m.iter().map(|r| r.iter().map(|x| x * x).sum::<f64>().sqrt()).product::<f64>();
        if !(d.abs() > 1e-9 * scale_value) {
            return None;
        }
        let b = [b0, b1, b2];
        let col = |k: usize| -> [[f64; 3]; 3] { std::array::from_fn(|r| std::array::from_fn(|c| if c == k { b[r] } else { m[r][c] })) };
        let p = [det(col(0)) / d, det(col(1)) / d, det(col(2)) / d];
        let x = [p[0] as f32, -(p[1] as f32), p[2] as f32, 1.0];
        *coin = std::array::from_fn(|r| (0..4).map(|k| view_proj[k][r] * x[k]).sum());
        if !coin.iter().all(|v| v.is_finite()) {
            return None;
        }
    }
    // Volume cut by the near plane (w = 1e-3): corners in front and points of the edges crossing it.
    const W: f32 = 1e-3;
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    let mut adds = |c: [f32; 4]| {
        let px = (c[0] / c[3] * 0.5 + 0.5) * size_value[0] as f32;
        let py = (0.5 - c[1] / c[3] * 0.5) * size_value[1] as f32;
        min = [min[0].min(px), min[1].min(py)];
        max = [max[0].max(px), max[1].max(py)];
    };
    for c in &corners {
        if c[3] > W {
            adds(*c);
        }
    }
    for i in 0..8usize {
        for bit in [1usize, 2, 4] {
            let j = i | bit;
            if j == i {
                continue;
            }
            let (a, b) = (corners[i], corners[j]);
            if (a[3] > W) != (b[3] > W) {
                let t = (W - a[3]) / (b[3] - a[3]);
                adds(std::array::from_fn(|k| a[k] + t * (b[k] - a[k])));
            }
        }
    }
    if !min[0].is_finite() {
        return Some([0, 0, 0, 0]);
    }
    let x0 = (min[0].floor() - 2.0).clamp(0.0, size_value[0] as f32) as u32;
    let y0 = (min[1].floor() - 2.0).clamp(0.0, size_value[1] as f32) as u32;
    let x1 = (max[0].ceil() + 2.0).clamp(0.0, size_value[0] as f32) as u32;
    let y1 = (max[1].ceil() + 2.0).clamp(0.0, size_value[1] as f32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return Some([0, 0, 0, 0]);
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

/// Screen rectangle of the polygon's vertices (or triangles), None if it touches the near plane.
pub fn rectangle_vertices(poly: &[[f32; 3]], view_proj: &[[f32; 4]; 4], size_value: [u32; 2]) -> Option<[u32; 4]> {
    if poly.is_empty() || size_value[0] == 0 || size_value[1] == 0 {
        return None;
    }
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for q in poly {
        let x = [q[0], -q[1], q[2], 1.0];
        let c: [f32; 4] = std::array::from_fn(|r| (0..4).map(|k| view_proj[k][r] * x[k]).sum());
        if !(c[3] > 1e-3) || !c.iter().all(|v| v.is_finite()) {
            return None;
        }
        let px = (c[0] / c[3] * 0.5 + 0.5) * size_value[0] as f32;
        let py = (0.5 - c[1] / c[3] * 0.5) * size_value[1] as f32;
        min = [min[0].min(px), min[1].min(py)];
        max = [max[0].max(px), max[1].max(py)];
    }
    let x0 = (min[0].floor() - 2.0).clamp(0.0, size_value[0] as f32) as u32;
    let y0 = (min[1].floor() - 2.0).clamp(0.0, size_value[1] as f32) as u32;
    let x1 = (max[0].ceil() + 2.0).clamp(0.0, size_value[0] as f32) as u32;
    let y1 = (max[1].ceil() + 2.0).clamp(0.0, size_value[1] as f32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return Some([0, 0, 0, 0]);
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

/// Intersection of two rectangles (None = whole screen).
pub fn inter(a: Option<[u32; 4]>, b: Option<[u32; 4]>) -> Option<[u32; 4]> {
    match (a, b) {
        (None, r) | (r, None) => r,
        (Some([ax, ay, aw, ah]), Some([bx, by, bw, bh])) => {
            let x0 = ax.max(bx);
            let y0 = ay.max(by);
            let x1 = (ax + aw).min(bx + bw);
            let y1 = (ay + ah).min(by + bh);
            Some(if x1 <= x0 || y1 <= y0 { [0, 0, 0, 0] } else { [x0, y0, x1 - x0, y1 - y0] })
        }
    }
}
