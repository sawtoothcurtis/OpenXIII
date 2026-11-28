//! ScaleFX 3x (passes 0 to 4), ported to the CPU from the libretro slang shaders
//! (edge-smoothing/scalefx). Settings: SFX_CLR 0.5, SFX_SAA 1, SFX_SCN 1. Texture edges are
//! clamped.
//!
//! ScaleFX: Copyright (c) 2016 Sp00kyFox - ScaleFX@web.de
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to deal
//! in the Software without restriction, including without limitation the rights
//! to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in
//! all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
//! THE SOFTWARE.

type V3 = [f32; 3];
type V4 = [f32; 4];

const SFX_CLR: f32 = 0.5;

struct Grid<T: Copy> {
    w: i64,
    h: i64,
    v: Vec<T>,
}

impl<T: Copy> Grid<T> {
    fn at(&self, x: i64, y: i64) -> T {
        self.v[(y.clamp(0, self.h - 1) * self.w + x.clamp(0, self.w - 1)) as usize]
    }
}

// Reference: http://www.compuphase.com/cmetric.htm
fn dist(a: V3, b: V3) -> f32 {
    let r = 0.5 * (a[0] + b[0]);
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let c = [2.0 + r, 4.0, 3.0 - r];
    (c[0] * d[0] * d[0] + c[1] * d[1] * d[1] + c[2] * d[2] * d[2]).sqrt() / 3.0
}

// corner strength (pass 1)
fn strength(d: f32, a: [f32; 2], b: [f32; 2]) -> f32 {
    let diff = a[0] - a[1];
    let wght1 = (SFX_CLR - d).max(0.0) / SFX_CLR;
    let wght2 = ((1.0 - d) + if a[0].min(b[0]) + a[0] > a[1].min(b[1]) + a[1] { diff } else { -diff }).clamp(0.0, 1.0);
    // SFX_SAA == 1
    (wght1 * wght2) * (a[0] * a[1])
}

fn ge(x: f32, y: f32) -> f32 {
    f32::from(u8::from(x > y))
}
fn le(x: f32, y: f32) -> f32 {
    f32::from(u8::from(x < y))
}
fn leq(x: f32, y: f32) -> f32 {
    f32::from(u8::from(x <= y))
}
fn not(x: f32) -> f32 {
    1.0 - x
}

// corner dominance at junctions: the four cells' (center, previous, next) components.
fn dom(c: [(f32, f32, f32); 4]) -> V4 {
    c.map(|(prev, centre, next)| 2.0 * centre - (prev + next))
}

// necessary but not sufficient junction condition for orthogonal edges
fn clear(crn: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    if crn[0] >= a[0].min(a[1]).max(b[0].min(b[1])) && crn[1] >= a[0].min(b[1]).max(b[0].min(a[1])) { 1.0 } else { 0.0 }
}

fn junction(jd: V4) -> V4 {
    let yzwx = [jd[1], jd[2], jd[3], jd[0]];
    let wxyz = [jd[3], jd[0], jd[1], jd[2]];
    let zwxy = [jd[2], jd[3], jd[0], jd[1]];
    std::array::from_fn(|k| (ge(jd[k], 0.0) * (leq(yzwx[k], 0.0) * leq(wxyz[k], 0.0) + ge(jd[k] + zwxy[k], yzwx[k] + wxyz[k]))).min(1.0))
}

/// ScaleFX 3x of a straight-alpha RGBA8 image: each output texel is a copy of a source texel.
fn scalefx(w: usize, h: usize, rgba: &[u8]) -> Vec<[u8; 4]> {
    let (wi, hi) = (w as i64, h as i64);
    let refpass = Grid { w: wi, h: hi, v: rgba.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect::<Vec<_>>() };
    // Metric colour: transparent texels take the key colour (magenta).
    let col = Grid {
        w: wi,
        h: hi,
        v: refpass.v.iter().map(|p| if p[3] < 128 { [1.0, 0.0, 1.0] } else { [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0] }).collect::<Vec<V3>>(),
    };
    fn each(wi: i64, hi: i64, f: impl Fn(i64, i64) -> V4 + Sync) -> Grid<V4> {
        let v = super::par_rows(hi as usize, |y| (0..wi).map(|x| f(x, y as i64)).collect());
        Grid { w: wi, h: hi, v }
    }
    // Pass 0: metric data. grid A B C / E F -> x y z / o w
    let m = each(wi, hi, |x, y| {
        let e = col.at(x, y);
        [dist(e, col.at(x - 1, y - 1)), dist(e, col.at(x, y - 1)), dist(e, col.at(x + 1, y - 1)), dist(e, col.at(x + 1, y))]
    });
    // Pass 1: corner strength.
    let s = each(wi, hi, |x, y| {
        let (a, b) = (m.at(x - 1, y - 1), m.at(x, y - 1));
        let (d, e, f) = (m.at(x - 1, y), m.at(x, y), m.at(x + 1, y));
        let (g, hh, i) = (m.at(x - 1, y + 1), m.at(x, y + 1), m.at(x + 1, y + 1));
        [
            strength(d[2], [d[3], e[1]], [a[3], d[1]]),
            strength(f[0], [e[3], e[1]], [b[3], f[1]]),
            strength(hh[2], [e[3], hh[1]], [hh[3], i[1]]),
            strength(hh[0], [d[3], hh[1]], [g[3], g[1]]),
        ]
    });
    // Pass 2: ambiguity resolution and edge level scan preparation, packed as 4 bits per
    // corner: corner, horizontal edge, vertical edge, orientation.
    let p2: Grid<[u8; 4]> = {
        let v = super::par_rows(h, |y| {
            let y = y as i64;
            (0..wi).map(|x| {
                let (a, b) = (m.at(x - 1, y - 1), m.at(x, y - 1));
                let (d, e, f) = (m.at(x - 1, y), m.at(x, y), m.at(x + 1, y));
                let (g, hh, i) = (m.at(x - 1, y + 1), m.at(x, y + 1), m.at(x + 1, y + 1));
                let (as_, bs, cs) = (s.at(x - 1, y - 1), s.at(x, y - 1), s.at(x + 1, y - 1));
                let (ds, es, fs) = (s.at(x - 1, y), s.at(x, y), s.at(x + 1, y));
                let (gs, hs, is) = (s.at(x - 1, y + 1), s.at(x, y + 1), s.at(x + 1, y + 1));
                // Junction of four cells (P.z, Q.w, R.x, S.y) and their dominance.
                let js = |p: V4, q: V4, r: V4, t: V4| [p[2], q[3], r[0], t[1]];
                let jdom = |p: V4, q: V4, r: V4, t: V4| dom([(p[1], p[2], p[3]), (q[2], q[3], q[0]), (r[3], r[0], r[1]), (t[0], t[1], t[2])]);
                let (jsx, jdx) = (js(as_, bs, es, ds), jdom(as_, bs, es, ds));
                let (jsy, jdy) = (js(bs, cs, fs, es), jdom(bs, cs, fs, es));
                let (jsz, jdz) = (js(es, fs, is, hs), jdom(es, fs, is, hs));
                let (jsw, jdw) = (js(ds, es, hs, gs), jdom(ds, es, hs, gs));
                let (jx, jy, jz, jw) = (junction(jdx), junction(jdy), junction(jdz), junction(jdw));
                let mut res = [
                    (jx[2] + not(jx[1]) * not(jx[3]) * ge(jsx[2], 0.0) * (jx[0] + ge(jsx[0] + jsx[2], jsx[1] + jsx[3]))).min(1.0),
                    (jy[3] + not(jy[2]) * not(jy[0]) * ge(jsy[3], 0.0) * (jy[1] + ge(jsy[1] + jsy[3], jsy[0] + jsy[2]))).min(1.0),
                    (jz[0] + not(jz[3]) * not(jz[1]) * ge(jsz[0], 0.0) * (jz[2] + ge(jsz[0] + jsz[2], jsz[1] + jsz[3]))).min(1.0),
                    (jw[1] + not(jw[0]) * not(jw[2]) * ge(jsw[1], 0.0) * (jw[3] + ge(jsw[1] + jsw[3], jsw[0] + jsw[2]))).min(1.0),
                ];
                // single pixel & end of line detection
                let keep = [jx[2], jy[3], jz[0], jw[1]];
                let r0 = res;
                for k in 0..4 {
                    res[k] = (r0[k] * (keep[k] + not(r0[(k + 3) % 4] * r0[(k + 1) % 4]))).min(1.0);
                }
                let clr = [
                    clear([d[2], e[0]], [d[3], e[1]], [a[3], d[1]]),
                    clear([f[0], e[2]], [e[3], e[1]], [b[3], f[1]]),
                    clear([hh[2], i[0]], [e[3], hh[1]], [hh[3], i[1]]),
                    clear([hh[0], g[2]], [d[3], hh[1]], [g[3], g[1]]),
                ];
                let hv = [d[3].min(a[3]), e[3].min(b[3]), e[3].min(hh[3]), d[3].min(g[3])];
                let vv = [e[1].min(d[1]), e[1].min(f[1]), hh[1].min(i[1]), hh[1].min(g[1])];
                let hadd = [d[3], e[3], e[3], d[3]];
                let vadd = [e[1], e[1], hh[1], hh[1]];
                std::array::from_fn(|k| {
                    let orien = ge(hv[k] + hadd[k], vv[k] + vadd[k]);
                    let hori = le(hv[k], vv[k]) * clr[k];
                    let vert = ge(hv[k], vv[k]) * clr[k];
                    (res[k] + 2.0 * hori + 4.0 * vert + 8.0 * orien).round() as u8
                })
            }).collect()
        });
        Grid { w: wi, h: hi, v }
    };
    // Pass 3: edge level scan, subpixel choice for the 4 corners and the 4 middles.
    type B4 = [bool; 4];
    let bits = |v: [u8; 4], bit: u8| -> B4 { v.map(|c| c >> bit & 1 == 1) };
    let choice: Vec<([u8; 4], [u8; 4])> = super::par_rows(h, |y| {
        let y = y as i64;
        (0..wi).map(|x| {
            let t = |dx: i64, dy: i64| p2.at(x + dx, y + dy);
            let (e, d, d0, d1, f, f0, f1) = (t(0, 0), t(-1, 0), t(-2, 0), t(-3, 0), t(1, 0), t(2, 0), t(3, 0));
            let (b, b0, b1, hh, h0, h1) = (t(0, -1), t(0, -2), t(0, -3), t(0, 1), t(0, 2), t(0, 3));
            let (ec, eh, ev, eo) = (bits(e, 0), bits(e, 1), bits(e, 2), bits(e, 3));
            let (dc, dh, do_, d0c, d0h, d1h) = (bits(d, 0), bits(d, 1), bits(d, 3), bits(d0, 0), bits(d0, 1), bits(d1, 1));
            let (fc, fh, fo, f0c, f0h, f1h) = (bits(f, 0), bits(f, 1), bits(f, 3), bits(f0, 0), bits(f0, 1), bits(f1, 1));
            let (bc, bv, bo, b0c, b0v, b1v) = (bits(b, 0), bits(b, 2), bits(b, 3), bits(b0, 0), bits(b0, 2), bits(b1, 2));
            let (hc, hv, ho, h0c, h0v, h1v) = (bits(hh, 0), bits(hh, 2), bits(hh, 3), bits(h0, 0), bits(h0, 2), bits(h1, 2));
            const X: usize = 0;
            const Y: usize = 1;
            const Z: usize = 2;
            const W: usize = 3;
            // lvl1 corners (SFX_SCN == 1)
            let lvl1x = ec[X];
            let lvl1y = ec[Y];
            let lvl1z = ec[Z];
            let lvl1w = ec[W];
            // lvl2 mid (left, right / up, down)
            let lvl2x = [(ec[X] && eh[Y]) && dc[Z], (ec[Y] && eh[X]) && fc[W]];
            let lvl2y = [(ec[Y] && ev[Z]) && bc[W], (ec[Z] && ev[Y]) && hc[X]];
            let lvl2z = [(ec[W] && eh[Z]) && dc[Y], (ec[Z] && eh[W]) && fc[X]];
            let lvl2w = [(ec[X] && ev[W]) && bc[Z], (ec[W] && ev[X]) && hc[Y]];
            // lvl3 corners (hori, vert)
            let lvl3x = [lvl2x[1] && (dh[Y] && dh[X]) && fh[Z], lvl2w[1] && (bv[W] && bv[X]) && hv[Z]];
            let lvl3y = [lvl2x[0] && (fh[X] && fh[Y]) && dh[W], lvl2y[1] && (bv[Z] && bv[Y]) && hv[W]];
            let lvl3z = [lvl2z[0] && (fh[W] && fh[Z]) && dh[X], lvl2y[0] && (hv[Y] && hv[Z]) && bv[X]];
            let lvl3w = [lvl2z[1] && (dh[Z] && dh[W]) && fh[Y], lvl2w[0] && (hv[X] && hv[W]) && bv[Y]];
            // lvl4 corners (hori, vert)
            let lvl4x = [
                (dc[X] && dh[Y] && eh[X] && eh[Y] && fh[X] && fh[Y]) && (d0c[Z] && d0h[W]),
                (bc[X] && bv[W] && ev[X] && ev[W] && hv[X] && hv[W]) && (b0c[Z] && b0v[Y]),
            ];
            let lvl4y = [
                (fc[Y] && fh[X] && eh[Y] && eh[X] && dh[Y] && dh[X]) && (f0c[W] && f0h[Z]),
                (bc[Y] && bv[Z] && ev[Y] && ev[Z] && hv[Y] && hv[Z]) && (b0c[W] && b0v[X]),
            ];
            let lvl4z = [
                (fc[Z] && fh[W] && eh[Z] && eh[W] && dh[Z] && dh[W]) && (f0c[X] && f0h[Y]),
                (hc[Z] && hv[Y] && ev[Z] && ev[Y] && bv[Z] && bv[Y]) && (h0c[X] && h0v[W]),
            ];
            let lvl4w = [
                (dc[W] && dh[Z] && eh[W] && eh[Z] && fh[W] && fh[Z]) && (d0c[Y] && d0h[X]),
                (hc[W] && hv[X] && ev[W] && ev[X] && bv[W] && bv[X]) && (h0c[Y] && h0v[Z]),
            ];
            // lvl5 mid (left, right / up, down)
            let lvl5x = [lvl4x[0] && (f0h[X] && f0h[Y]) && (d1h[Z] && d1h[W]), lvl4y[0] && (d0h[Y] && d0h[X]) && (f1h[W] && f1h[Z])];
            let lvl5y = [lvl4y[1] && (h0v[Y] && h0v[Z]) && (b1v[W] && b1v[X]), lvl4z[1] && (b0v[Z] && b0v[Y]) && (h1v[X] && h1v[W])];
            let lvl5z = [lvl4w[0] && (f0h[W] && f0h[Z]) && (d1h[Y] && d1h[X]), lvl4z[0] && (d0h[Z] && d0h[W]) && (f1h[X] && f1h[Y])];
            let lvl5w = [lvl4x[1] && (h0v[X] && h0v[W]) && (b1v[Z] && b1v[Y]), lvl4w[1] && (b0v[W] && b0v[X]) && (h1v[Y] && h1v[Z])];
            // lvl6 corners (hori, vert)
            let lvl6x = [lvl5x[1] && (d1h[Y] && d1h[X]), lvl5w[1] && (b1v[W] && b1v[X])];
            let lvl6y = [lvl5x[0] && (f1h[X] && f1h[Y]), lvl5y[1] && (b1v[Z] && b1v[Y])];
            let lvl6z = [lvl5z[0] && (f1h[W] && f1h[Z]), lvl5y[0] && (h1v[Y] && h1v[Z])];
            let lvl6w = [lvl5z[1] && (d1h[Z] && d1h[W]), lvl5w[0] && (h1v[X] && h1v[W])];
            // subpixels - 0 = E, 1 = D, 2 = D0, 3 = F, 4 = F0, 5 = B, 6 = B0, 7 = H, 8 = H0
            let sel = |c: &[(bool, u8)]| c.iter().find(|(b, _)| *b).map_or(0, |(_, v)| *v);
            let crn = [
                sel(&[
                    (lvl1x && eo[X] || lvl3x[0] && eo[Y] || lvl4x[0] && do_[X] || lvl6x[0] && fo[Y], 5),
                    (lvl1x || lvl3x[1] && !eo[W] || lvl4x[1] && !bo[X] || lvl6x[1] && !ho[W], 1),
                    (lvl3x[0], 3), (lvl3x[1], 7), (lvl4x[0], 2), (lvl4x[1], 6), (lvl6x[0], 4), (lvl6x[1], 8),
                ]),
                sel(&[
                    (lvl1y && eo[Y] || lvl3y[0] && eo[X] || lvl4y[0] && fo[Y] || lvl6y[0] && do_[X], 5),
                    (lvl1y || lvl3y[1] && !eo[Z] || lvl4y[1] && !bo[Y] || lvl6y[1] && !ho[Z], 3),
                    (lvl3y[0], 1), (lvl3y[1], 7), (lvl4y[0], 4), (lvl4y[1], 6), (lvl6y[0], 2), (lvl6y[1], 8),
                ]),
                sel(&[
                    (lvl1z && eo[Z] || lvl3z[0] && eo[W] || lvl4z[0] && fo[Z] || lvl6z[0] && do_[W], 7),
                    (lvl1z || lvl3z[1] && !eo[Y] || lvl4z[1] && !ho[Z] || lvl6z[1] && !bo[Y], 3),
                    (lvl3z[0], 1), (lvl3z[1], 5), (lvl4z[0], 4), (lvl4z[1], 8), (lvl6z[0], 2), (lvl6z[1], 6),
                ]),
                sel(&[
                    (lvl1w && eo[W] || lvl3w[0] && eo[Z] || lvl4w[0] && do_[W] || lvl6w[0] && fo[Z], 7),
                    (lvl1w || lvl3w[1] && !eo[X] || lvl4w[1] && !ho[W] || lvl6w[1] && !bo[X], 1),
                    (lvl3w[0], 3), (lvl3w[1], 5), (lvl4w[0], 2), (lvl4w[1], 8), (lvl6w[0], 4), (lvl6w[1], 6),
                ]),
            ];
            let mid = [
                sel(&[
                    (lvl2x[0] && eo[X] || lvl2x[1] && eo[Y] || lvl5x[0] && do_[X] || lvl5x[1] && fo[Y], 5),
                    (lvl2x[0], 1), (lvl2x[1], 3), (lvl5x[0], 2), (lvl5x[1], 4),
                    (ec[X] && dc[Z] && ec[Y] && fc[W], if eo[X] { if eo[Y] { 5 } else { 3 } } else { 1 }),
                ]),
                sel(&[
                    (lvl2y[0] && !eo[Y] || lvl2y[1] && !eo[Z] || lvl5y[0] && !bo[Y] || lvl5y[1] && !ho[Z], 3),
                    (lvl2y[0], 5), (lvl2y[1], 7), (lvl5y[0], 6), (lvl5y[1], 8),
                    (ec[Y] && bc[W] && ec[Z] && hc[X], if !eo[Y] { if !eo[Z] { 3 } else { 7 } } else { 5 }),
                ]),
                sel(&[
                    (lvl2z[0] && eo[W] || lvl2z[1] && eo[Z] || lvl5z[0] && do_[W] || lvl5z[1] && fo[Z], 7),
                    (lvl2z[0], 1), (lvl2z[1], 3), (lvl5z[0], 2), (lvl5z[1], 4),
                    (ec[Z] && fc[X] && ec[W] && dc[Y], if eo[Z] { if eo[W] { 7 } else { 1 } } else { 3 }),
                ]),
                sel(&[
                    (lvl2w[0] && !eo[X] || lvl2w[1] && !eo[W] || lvl5w[0] && !bo[X] || lvl5w[1] && !ho[W], 1),
                    (lvl2w[0], 5), (lvl2w[1], 7), (lvl5w[0], 6), (lvl5w[1], 8),
                    (ec[W] && hc[Y] && ec[X] && bc[Z], if !eo[W] { if !eo[X] { 1 } else { 5 } } else { 7 }),
                ]),
            ];
            (crn, mid)
        }).collect()
    });
    // Pass 4: 3x output, each subpixel copies the chosen source texel.
    let (ow, oh) = (w * 3, h * 3);
    let mut out = Vec::with_capacity(ow * oh);
    for oy in 0..oh {
        for ox in 0..ow {
            let (x, y, fx, fy) = (ox / 3, oy / 3, ox % 3, oy % 3);
            let (crn, mid) = choice[y * w + x];
            let sp = match (fy, fx) {
                (0, 0) => crn[0],
                (0, 1) => mid[0],
                (0, _) => crn[1],
                (1, 0) => mid[3],
                (1, 1) => 0,
                (1, _) => mid[1],
                (_, 0) => crn[3],
                (_, 1) => mid[2],
                _ => crn[2],
            };
            let (dx, dy) = [(0, 0), (-1, 0), (-2, 0), (1, 0), (2, 0), (0, -1), (0, -2), (0, 1), (0, 2)][sp as usize];
            out.push(refpass.at(x as i64 + dx, y as i64 + dy));
        }
    }
    out
}

/// ScaleFX alone: 3x of a straight-alpha RGBA8 image (only colours of the source).
pub fn scalefx3x(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    scalefx(w, h, rgba).into_iter().flatten().collect()
}
