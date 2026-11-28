//! SMAA 1x (luma edge detection, blending weight calculation, neighbourhood blending), ported to
//! the CPU from SMAA.hlsl (github.com/iryoku/smaa), ULTRA preset, with its precomputed area and
//! search textures (smaa-area.bin and smaa-search.bin: AreaTex.h and SearchTex.h as raw bytes).
//! Coordinates are in texels (texel centres at +0.5); bilinear fetches clamp at the edges.
//!
//! Copyright (C) 2013 Jorge Jimenez (jorge@iryoku.com)
//! Copyright (C) 2013 Jose I. Echevarria (joseignacioechevarria@gmail.com)
//! Copyright (C) 2013 Belen Masia (bmasia@unizar.es)
//! Copyright (C) 2013 Fernando Navarro (fernandn@microsoft.com)
//! Copyright (C) 2013 Diego Gutierrez (diegog@unizar.es)
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! this software and associated documentation files (the "Software"), to deal in
//! the Software without restriction, including without limitation the rights to
//! use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
//! of the Software, and to permit persons to whom the Software is furnished to
//! do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in
//! all copies or substantial portions of the Software. As clarification, there
//! is no requirement that the copyright notice and permission be included in
//! binary distributions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//! SOFTWARE.

static AREA_TEX: &[u8] = include_bytes!("smaa-area.bin");
static SEARCH_TEX: &[u8] = include_bytes!("smaa-search.bin");

// SMAA_PRESET_ULTRA
const THRESHOLD: f32 = 0.05;
const MAX_SEARCH_STEPS: f32 = 32.0;
const MAX_SEARCH_STEPS_DIAG: f32 = 16.0;
const CORNER_ROUNDING_NORM: f32 = 25.0 / 100.0;
const LOCAL_CONTRAST_ADAPTATION_FACTOR: f32 = 2.0;
const AREATEX_MAX_DISTANCE: f32 = 16.0;
const AREATEX_MAX_DISTANCE_DIAG: f32 = 20.0;
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Image of N float channels.
struct Tex<const N: usize> {
    w: i64,
    h: i64,
    v: Vec<[f32; N]>,
}

impl<const N: usize> Tex<N> {
    fn texel(&self, x: i64, y: i64) -> [f32; N] {
        self.v[(y.clamp(0, self.h - 1) * self.w + x.clamp(0, self.w - 1)) as usize]
    }

    /// Bilinear fetch at texel coordinates (centres at +0.5), edges clamped.
    fn bilinear(&self, x: f32, y: f32) -> [f32; N] {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let (a, b, c, d) = (self.texel(x0, y0), self.texel(x0 + 1, y0), self.texel(x0, y0 + 1), self.texel(x0 + 1, y0 + 1));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            top + (bottom - top) * ty
        })
    }
}

fn area_tex() -> Tex<2> {
    Tex { w: 160, h: 560, v: AREA_TEX.chunks_exact(2).map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0]).collect() }
}

fn search_tex() -> Tex<1> {
    Tex { w: 64, h: 16, v: SEARCH_TEX.iter().map(|&p| [p as f32 / 255.0]).collect() }
}

fn round(v: f32) -> f32 {
    v.round()
}

/// First pass: luma edge detection (r: left edge, g: top edge).
fn edges(luma: &Tex<1>) -> Tex<2> {
    let v = super::par_rows(luma.h as usize, |y| {
        let y = y as i64;
        (0..luma.w).map(|x| {
            let l = |dx: i64, dy: i64| luma.texel(x + dx, y + dy)[0];
            let (lc, lleft, ltop) = (l(0, 0), l(-1, 0), l(0, -1));
            let delta_xy = [(lc - lleft).abs(), (lc - ltop).abs()];
            let mut e = delta_xy.map(|d| if d >= THRESHOLD { 1.0 } else { 0.0 });
            if e[0] + e[1] == 0.0 {
                return [0.0, 0.0];
            }
            let (lright, lbottom) = (l(1, 0), l(0, 1));
            let delta_zw = [(lc - lright).abs(), (lc - lbottom).abs()];
            let max_delta = [delta_xy[0].max(delta_zw[0]), delta_xy[1].max(delta_zw[1])];
            let (lll, ltt) = (l(-2, 0), l(0, -2));
            let delta_zw = [(lleft - lll).abs(), (ltop - ltt).abs()];
            let max_delta = [max_delta[0].max(delta_zw[0]), max_delta[1].max(delta_zw[1])];
            let final_delta = max_delta[0].max(max_delta[1]);
            for k in 0..2 {
                if LOCAL_CONTRAST_ADAPTATION_FACTOR * delta_xy[k] < final_delta {
                    e[k] = 0.0;
                }
            }
            e
        }).collect()
    });
    Tex { w: luma.w, h: luma.h, v }
}

struct Weights<'a> {
    e: &'a Tex<2>,
    area: &'a Tex<2>,
    search: &'a Tex<1>,
}

fn decode_diag(e: [f32; 2]) -> [f32; 2] {
    [round(e[0] * (5.0 * e[0] - 5.0 * 0.75).abs()), round(e[1])]
}

impl Weights<'_> {
    fn sample(&self, x: f32, y: f32) -> [f32; 2] {
        self.e.bilinear(x, y)
    }

    fn search_diag1(&self, tc: [f32; 2], dir: [f32; 2], e: &mut [f32; 2]) -> [f32; 2] {
        let mut c = [tc[0], tc[1], -1.0, 1.0];
        while c[2] < MAX_SEARCH_STEPS_DIAG - 1.0 && c[3] > 0.9 {
            c[0] += dir[0];
            c[1] += dir[1];
            c[2] += 1.0;
            *e = self.sample(c[0], c[1]);
            c[3] = 0.5 * (e[0] + e[1]);
        }
        [c[2], c[3]]
    }

    fn search_diag2(&self, tc: [f32; 2], dir: [f32; 2], e: &mut [f32; 2]) -> [f32; 2] {
        let mut c = [tc[0] + 0.25, tc[1], -1.0, 1.0];
        while c[2] < MAX_SEARCH_STEPS_DIAG - 1.0 && c[3] > 0.9 {
            c[0] += dir[0];
            c[1] += dir[1];
            c[2] += 1.0;
            *e = decode_diag(self.sample(c[0], c[1]));
            c[3] = 0.5 * (e[0] + e[1]);
        }
        [c[2], c[3]]
    }

    fn area_diag(&self, dist: [f32; 2], e: [f32; 2], offset: f32) -> [f32; 2] {
        let x = AREATEX_MAX_DISTANCE_DIAG * e[0] + dist[0] + 0.5 + 80.0;
        let y = AREATEX_MAX_DISTANCE_DIAG * e[1] + dist[1] + 0.5 + 80.0 * offset;
        self.area.bilinear(x, y)
    }

    fn diag_weights(&self, tc: [f32; 2], e: [f32; 2]) -> [f32; 2] {
        let mut weights = [0.0f32; 2];
        let mut end = [0.0f32; 2];
        // d = (x, y, z, w)
        let mut d = [0.0f32; 4];
        if e[0] > 0.0 {
            let r = self.search_diag1(tc, [-1.0, 1.0], &mut end);
            d[0] = r[0] + f32::from(u8::from(end[1] > 0.9));
            d[2] = r[1];
        }
        let r = self.search_diag1(tc, [1.0, -1.0], &mut end);
        d[1] = r[0];
        d[3] = r[1];
        if d[0] + d[1] > 2.0 {
            let coords = [tc[0] - d[0] + 0.25, tc[1] + d[0], tc[0] + d[1], tc[1] - d[1] - 0.25];
            let a = self.sample(coords[0] - 1.0, coords[1]);
            let b = self.sample(coords[2] + 1.0, coords[3]);
            let raw = [a[0], a[1], b[0], b[1]];
            // SMAADecodeDiagBilinearAccess(float4): r and b channels unpacked, then rounded.
            let dec = [round(raw[0] * (5.0 * raw[0] - 3.75).abs()), round(raw[1]), round(raw[2] * (5.0 * raw[2] - 3.75).abs()), round(raw[3])];
            // c.yxwz = decoded
            let c = [dec[1], dec[0], dec[3], dec[2]];
            let mut cc = [2.0 * c[0] + c[1], 2.0 * c[2] + c[3]];
            if d[2] >= 0.9 {
                cc[0] = 0.0;
            }
            if d[3] >= 0.9 {
                cc[1] = 0.0;
            }
            let a = self.area_diag([d[0], d[1]], cc, 0.0);
            weights[0] += a[0];
            weights[1] += a[1];
        }
        let r = self.search_diag2(tc, [-1.0, -1.0], &mut end);
        d[0] = r[0];
        d[2] = r[1];
        if self.sample(tc[0] + 1.0, tc[1])[0] > 0.0 {
            let r = self.search_diag2(tc, [1.0, 1.0], &mut end);
            d[1] = r[0] + f32::from(u8::from(end[1] > 0.9));
            d[3] = r[1];
        } else {
            d[1] = 0.0;
            d[3] = 0.0;
        }
        if d[0] + d[1] > 2.0 {
            let coords = [tc[0] - d[0], tc[1] - d[0], tc[0] + d[1], tc[1] + d[1]];
            let cx = self.sample(coords[0] - 1.0, coords[1])[1];
            let cy = self.sample(coords[0], coords[1] - 1.0)[0];
            let s = self.sample(coords[2] + 1.0, coords[3]);
            let (cz, cw) = (s[1], s[0]);
            let mut cc = [2.0 * cx + cy, 2.0 * cz + cw];
            if d[2] >= 0.9 {
                cc[0] = 0.0;
            }
            if d[3] >= 0.9 {
                cc[1] = 0.0;
            }
            let a = self.area_diag([d[0], d[1]], cc, 0.0);
            weights[0] += a[1];
            weights[1] += a[0];
        }
        weights
    }

    fn search_length(&self, e: [f32; 2], offset: f32) -> f32 {
        // Texel coordinates in the packed (64 x 16) search texture.
        let x = 32.0 * e[0] + 66.0 * offset + 0.5;
        let y = -32.0 * e[1] + 32.5;
        self.search.bilinear(x, y)[0]
    }

    fn search_x_left(&self, mut tc: [f32; 2], end: f32) -> f32 {
        let mut e = [0.0f32, 1.0];
        while tc[0] > end && e[1] > 0.8281 && e[0] == 0.0 {
            e = self.sample(tc[0], tc[1]);
            tc[0] -= 2.0;
        }
        let offset = -(255.0 / 127.0) * self.search_length(e, 0.0) + 3.25;
        tc[0] + offset
    }

    fn search_x_right(&self, mut tc: [f32; 2], end: f32) -> f32 {
        let mut e = [0.0f32, 1.0];
        while tc[0] < end && e[1] > 0.8281 && e[0] == 0.0 {
            e = self.sample(tc[0], tc[1]);
            tc[0] += 2.0;
        }
        let offset = -(255.0 / 127.0) * self.search_length(e, 0.5) + 3.25;
        tc[0] - offset
    }

    fn search_y_up(&self, mut tc: [f32; 2], end: f32) -> f32 {
        let mut e = [1.0f32, 0.0];
        while tc[1] > end && e[0] > 0.8281 && e[1] == 0.0 {
            e = self.sample(tc[0], tc[1]);
            tc[1] -= 2.0;
        }
        let offset = -(255.0 / 127.0) * self.search_length([e[1], e[0]], 0.0) + 3.25;
        tc[1] + offset
    }

    fn search_y_down(&self, mut tc: [f32; 2], end: f32) -> f32 {
        let mut e = [1.0f32, 0.0];
        while tc[1] < end && e[0] > 0.8281 && e[1] == 0.0 {
            e = self.sample(tc[0], tc[1]);
            tc[1] += 2.0;
        }
        let offset = -(255.0 / 127.0) * self.search_length([e[1], e[0]], 0.5) + 3.25;
        tc[1] - offset
    }

    fn area(&self, dist: [f32; 2], e1: f32, e2: f32, offset: f32) -> [f32; 2] {
        let x = AREATEX_MAX_DISTANCE * round(4.0 * e1) + dist[0] + 0.5;
        let y = AREATEX_MAX_DISTANCE * round(4.0 * e2) + dist[1] + 0.5 + 80.0 * offset;
        self.area.bilinear(x, y)
    }

    fn corner_rounding(d: [f32; 2]) -> [f32; 2] {
        // step(d.xy, d.yx)
        let lr = [f32::from(u8::from(d[1] >= d[0])), f32::from(u8::from(d[0] >= d[1]))];
        let s = lr[0] + lr[1];
        [(1.0 - CORNER_ROUNDING_NORM) * lr[0] / s, (1.0 - CORNER_ROUNDING_NORM) * lr[1] / s]
    }

    fn horizontal_corner(&self, w: &mut [f32; 2], tc: [f32; 4], d: [f32; 2]) {
        let r = Self::corner_rounding(d);
        let mut f = [1.0f32, 1.0];
        f[0] -= r[0] * self.sample(tc[0], tc[1] + 1.0)[0];
        f[0] -= r[1] * self.sample(tc[2] + 1.0, tc[3] + 1.0)[0];
        f[1] -= r[0] * self.sample(tc[0], tc[1] - 2.0)[0];
        f[1] -= r[1] * self.sample(tc[2] + 1.0, tc[3] - 2.0)[0];
        w[0] *= f[0].clamp(0.0, 1.0);
        w[1] *= f[1].clamp(0.0, 1.0);
    }

    fn vertical_corner(&self, w: &mut [f32; 2], tc: [f32; 4], d: [f32; 2]) {
        let r = Self::corner_rounding(d);
        let mut f = [1.0f32, 1.0];
        f[0] -= r[0] * self.sample(tc[0] + 1.0, tc[1])[1];
        f[0] -= r[1] * self.sample(tc[2] + 1.0, tc[3] + 1.0)[1];
        f[1] -= r[0] * self.sample(tc[0] - 2.0, tc[1])[1];
        f[1] -= r[1] * self.sample(tc[2] - 2.0, tc[3] + 1.0)[1];
        w[0] *= f[0].clamp(0.0, 1.0);
        w[1] *= f[1].clamp(0.0, 1.0);
    }

    /// Second pass for the texel (x, y).
    fn pixel(&self, x: i64, y: i64) -> [f32; 4] {
        let tc = [x as f32 + 0.5, y as f32 + 0.5];
        let off0 = [tc[0] - 0.25, tc[1] - 0.125, tc[0] + 1.25, tc[1] - 0.125];
        let off1 = [tc[0] - 0.125, tc[1] - 0.25, tc[0] - 0.125, tc[1] + 1.25];
        let off2 = [off0[0] - 2.0 * MAX_SEARCH_STEPS, off0[2] + 2.0 * MAX_SEARCH_STEPS, off1[1] - 2.0 * MAX_SEARCH_STEPS, off1[3] + 2.0 * MAX_SEARCH_STEPS];
        let mut weights = [0.0f32; 4];
        let mut e = self.e.texel(x, y);
        if e[1] > 0.0 {
            let dw = self.diag_weights(tc, e);
            weights[0] = dw[0];
            weights[1] = dw[1];
            if weights[0] == -weights[1] {
                let cx = self.search_x_left([off0[0], off0[1]], off2[0]);
                let cy = off1[1];
                let e1 = self.sample(cx, cy)[0];
                let cz = self.search_x_right([off0[2], off0[3]], off2[1]);
                let d = [round(cx - tc[0]).abs(), round(cz - tc[0]).abs()];
                let sqrt_d = [d[0].sqrt(), d[1].sqrt()];
                let e2 = self.sample(cz + 1.0, cy)[0];
                let mut rg = self.area(sqrt_d, e1, e2, 0.0);
                self.horizontal_corner(&mut rg, [cx, tc[1], cz, tc[1]], d);
                weights[0] = rg[0];
                weights[1] = rg[1];
            } else {
                e[0] = 0.0;
            }
        }
        if e[0] > 0.0 {
            let cy = self.search_y_up([off1[0], off1[1]], off2[2]);
            let cx = off0[0];
            let e1 = self.sample(cx, cy)[1];
            let cz = self.search_y_down([off1[2], off1[3]], off2[3]);
            let d = [round(cy - tc[1]).abs(), round(cz - tc[1]).abs()];
            let sqrt_d = [d[0].sqrt(), d[1].sqrt()];
            let e2 = self.sample(cx, cz + 1.0)[1];
            let mut ba = self.area(sqrt_d, e1, e2, 0.0);
            self.vertical_corner(&mut ba, [tc[0], cy, tc[0], cz], d);
            weights[2] = ba[0];
            weights[3] = ba[1];
        }
        weights
    }
}

/// Third pass on one image with the blending weights.
fn blend<const N: usize>(img: &Tex<N>, bw: &Tex<4>) -> Vec<[f32; N]> {
    super::par_rows(img.h as usize, |y| {
        let y = y as i64;
        (0..img.w).map(|x| {
            let here = bw.texel(x, y);
            let a = [bw.texel(x + 1, y)[3], bw.texel(x, y + 1)[1], here[2], here[0]];
            if a.iter().sum::<f32>() < 1e-5 {
                return img.texel(x, y);
            }
            let tc = [x as f32 + 0.5, y as f32 + 0.5];
            let h = a[0].max(a[2]) > a[1].max(a[3]);
            let (off, mut wgt) = if h { ([a[0], 0.0, a[2], 0.0], [a[0], a[2]]) } else { ([0.0, a[1], 0.0, a[3]], [a[1], a[3]]) };
            let s = wgt[0] + wgt[1];
            wgt = [wgt[0] / s, wgt[1] / s];
            let c1 = img.bilinear(tc[0] + off[0], tc[1] + off[1]);
            let c2 = img.bilinear(tc[0] - off[2], tc[1] - off[3]);
            std::array::from_fn(|k| wgt[0] * c1[k] + wgt[1] * c2[k])
        }).collect()
    })
}

/// SMAA on an image already enlarged (nearest neighbour), straight-alpha RGBA8: edges found on
/// the colour composed over a magenta key; the colour (transparent texels filled with the
/// nearest visible colour) and the alpha (as a grey image) are blended with the same weights.
pub fn antialias(w: usize, h: usize, rgba: &[u8], filled_rgb: &[[f32; 3]]) -> Vec<u8> {
    let (wi, hi) = (w as i64, h as i64);
    let luma = Tex {
        w: wi,
        h: hi,
        v: rgba
            .chunks_exact(4)
            .map(|p| {
                let a = p[3] as f32 / 255.0;
                let c = |v: u8, key: f32| v as f32 / 255.0 * a + key * (1.0 - a);
                [LUMA[0] * c(p[0], 1.0) + LUMA[1] * c(p[1], 0.0) + LUMA[2] * c(p[2], 1.0)]
            })
            .collect(),
    };
    let e = edges(&luma);
    let (area, search) = (area_tex(), search_tex());
    let wc = Weights { e: &e, area: &area, search: &search };
    let bw = super::par_rows(h, |y| (0..wi).map(|x| wc.pixel(x, y as i64)).collect());
    let bw = Tex { w: wi, h: hi, v: bw };
    let colour = blend(&Tex { w: wi, h: hi, v: filled_rgb.to_vec() }, &bw);
    let alpha = blend(&Tex { w: wi, h: hi, v: rgba.chunks_exact(4).map(|p| [p[3] as f32 / 255.0]).collect() }, &bw);
    let q = |v: f32| (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    colour.iter().zip(&alpha).flat_map(|(c, a)| [q(c[0]), q(c[1]), q(c[2]), q(a[0])]).collect()
}
