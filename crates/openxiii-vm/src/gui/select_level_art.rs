//! Improved build only: the two pictures of the Select level page, made at run time from the
//! player's own game files (nothing derived from the game is shipped: only the parameters below).
//!
//! - Major Jones in her Christmas hat: the four quarters XIIIMenu.pause1Noel..pause4Noel (256 × 256
//!   each, DXT1 without any transparent texel, no mask texture and no script of the PC version
//!   draws them; laid out as a 2 × 2 grid, 1 2 / 3 4), whose flat magenta background is removed
//!   by a chroma key: the "PAUSE..." text erased, the background found by a flood fill from the
//!   borders, then a soft alpha worked out only in a thin band along it (mix of the key and the
//!   nearest solid colour, the black outline kept whole), a despill and the specks dropped.
//!   Mirrored, 10 rows cut off the bottom; `jones_tile` stands her on the picture's frame.
//! - The background: frame 2000 of Cine14.bik (the lift shaft seen from above, left panel of the
//!   split screen), cropped inside the panel so no black bar remains.
//!
//! Both are made once per run (memory) and can be kept in a disk cache folder given by the caller.
//!
//! The cut-out's edge is hard (and so is the mask of the menus' other characters, menu_figures.rs):
//! `smooth_edge` makes the picture at the size it is shown at on the screen with its edge
//! anti-aliased, the shape and the inside unchanged.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::menu_boxes::Image;

/// Textures of the four quarters, in grid order (top left, top right, bottom left, bottom right).
pub const JONES_TEXTURES: [&str; 4] = ["XIIIMenu.pause1Noel", "XIIIMenu.pause2Noel", "XIIIMenu.pause3Noel", "XIIIMenu.pause4Noel"];
/// Size of each quarter; any other size gives no character.
pub const QUARTER: u32 = 256;
/// Background colour expected at the top left corner (the texture's flat magenta); the colour
/// actually read there is used, provided it is within `KEY_CHECK` of this one.
pub const KEY_EXPECTED: [u8; 3] = [189, 36, 132];
const KEY_CHECK: f32 = 24.0;
/// Colour distance (Euclidean, 0-255 per channel) under which a texel joins the background fill.
pub const TOLERANCE: f32 = 40.0;
/// Width of the edge band (texels from the filled background) where alpha is worked out; deeper
/// texels stay solid, so key-like colours inside the character are never touched.
pub const BAND: f32 = 2.5;
/// Half size of the window the candidate solid colours of an edge texel are taken from.
const CANDIDATE_WINDOW: i32 = 6;
/// A candidate solid colour is at least this far from the key.
const SOLID_MIN: f32 = 80.0;
/// Two-threshold ramp on the colour distance to the key: alpha 0 under the first, 1 over the
/// second. Every edge texel is at most this transparent when its solid colour is dark (outline).
pub const RAMP: [f32; 2] = [30.0, 85.0];
/// Luminance under which a solid colour counts as the dark outline.
const DARK: f32 = 70.0;
/// Groups of fewer opaque texels than this, cut off from the character, are dropped.
const SPECK: usize = 24;
/// Rectangles of the assembled 512 × 512 picture painted with the background colour before the
/// fill: the "PAUSE..." text (x, y, w, h). The character starts 18 texels right of it.
pub const ERASE_RECTS: [[u32; 4]; 1] = [[18, 380, 132, 34]];
/// Extra fill seeds (x, y) for background pockets the borders do not reach (none needed: the gap
/// between the legs opens on the bottom edge).
pub const EXTRA_SEEDS: [[u32; 2]; 0] = [];
/// Below this alpha an edge texel is dropped.
const ALPHA_MIN: f32 = 0.08;
/// Texels within this distance of the background lose any magenta cast (min(R, B) above G)
/// beyond that of the solid colours around them; over it by `CAST_MARGIN`, they are unmixed.
pub const DESPILL: f32 = 5.0;
const CAST_MARGIN: f32 = 10.0;
/// Rows cut off the bottom (in the source resolution): the quarters end on a ragged line.
pub const CUT_BOTTOM: u32 = 10;

/// Video and frame of the background, and the window kept (x, y, w, h) in its 640 × 480 frame:
/// the left panel spans x 0-246, y 96-371 with 1-2 blended rows at its edges; 2 texels of margin.
pub const BACKGROUND_VIDEO: &str = "Cine14";
pub const BACKGROUND_FRAME: u32 = 2000;
pub const BACKGROUND_FRAME_SIZE: (u32, u32) = (640, 480);
pub const BACKGROUND_CROP: [u32; 4] = [0, 98, 245, 272];

/// Placement of Jones against the background picture (from the mock-up): her figure (opaque
/// texels) is this many background heights tall, her lowest texels on the background's bottom
/// row, and her figure's left edge this many background heights left of the background's right edge.
pub const FIGURE_HEIGHT: f32 = 0.921;
pub const FIGURE_OVERLAP: f32 = 0.087;

/// Bumped whenever the making changes, so a disk cache from an older build is not reused.
const ART_VERSION: u32 = 9;

fn at(img: &Image, x: u32, y: u32) -> usize {
    ((y * img.w + x) * 4) as usize
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn rgb(img: &Image, i: usize) -> [f32; 3] {
    [img.rgba[i] as f32, img.rgba[i + 1] as f32, img.rgba[i + 2] as f32]
}

/// The four quarters laid out 1 2 / 3 4. None unless all four are `QUARTER` square.
pub fn assemble(quarters: &[Image; 4]) -> Option<Image> {
    assemble_sized(quarters, QUARTER)
}

fn assemble_sized(quarters: &[Image; 4], q: u32) -> Option<Image> {
    if quarters.iter().any(|t| t.w != q || t.h != q || t.rgba.len() != (q * q * 4) as usize) {
        return None;
    }
    let mut out = Image::new(q * 2, q * 2);
    for (n, t) in quarters.iter().enumerate() {
        let (ox, oy) = ((n as u32 % 2) * q, (n as u32 / 2) * q);
        for y in 0..q {
            let src = (y * q * 4) as usize;
            let dst = at(&out, ox, oy + y);
            out.rgba[dst..dst + (q * 4) as usize].copy_from_slice(&t.rgba[src..src + (q * 4) as usize]);
        }
    }
    Some(out)
}

/// Paints rectangles (x, y, w, h; clipped) with an opaque colour.
pub fn paint_rects(img: &mut Image, rects: &[[u32; 4]], colour: [u8; 3]) {
    for &[x, y, w, h] in rects {
        for yy in y.min(img.h)..(y + h).min(img.h) {
            for xx in x.min(img.w)..(x + w).min(img.w) {
                let i = at(img, xx, yy);
                img.rgba[i..i + 4].copy_from_slice(&[colour[0], colour[1], colour[2], 255]);
            }
        }
    }
}

/// Background mask: 4-connected flood fill over texels within `tolerance` of `key`, from every
/// border texel and from `seeds`.
pub fn flood_background(img: &Image, key: [u8; 3], tolerance: f32, seeds: &[[u32; 2]]) -> Vec<bool> {
    let (w, h) = (img.w as usize, img.h as usize);
    let key = key.map(f32::from);
    let mut bg = vec![false; w * h];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for x in 0..w {
        stack.push((x, 0));
        stack.push((x, h.saturating_sub(1)));
    }
    for y in 0..h {
        stack.push((0, y));
        stack.push((w.saturating_sub(1), y));
    }
    stack.extend(seeds.iter().map(|&[x, y]| (x as usize, y as usize)));
    while let Some((x, y)) = stack.pop() {
        if x >= w || y >= h || bg[y * w + x] || dist(rgb(img, (y * w + x) * 4), key) >= tolerance {
            continue;
        }
        bg[y * w + x] = true;
        if x > 0 {
            stack.push((x - 1, y));
        }
        if y > 0 {
            stack.push((x, y - 1));
        }
        stack.push((x + 1, y));
        stack.push((x, y + 1));
    }
    bg
}

/// Euclidean distance of each texel to the nearest background texel, looked for up to `radius`
/// texels away (radius + 1 beyond; 0 on the background).
fn distance_to_background(bg: &[bool], w: usize, h: usize, radius: i32) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            if bg[y * w + x] {
                continue;
            }
            let mut best = (radius + 1) as f32;
            for dy in -radius..=radius {
                let yy = y as i32 + dy;
                if yy < 0 || yy >= h as i32 {
                    continue;
                }
                for dx in -radius..=radius {
                    let xx = x as i32 + dx;
                    if xx >= 0 && xx < w as i32 && bg[yy as usize * w + xx as usize] {
                        best = best.min(((dx * dx + dy * dy) as f32).sqrt());
                    }
                }
            }
            out[y * w + x] = best;
        }
    }
    out
}

/// Removes the background `bg` (colour `key`), as a chroma key limited to the texels connected to
/// it: transparent on `bg`, untouched deeper than `DESPILL` inside the character. Each texel within
/// `BAND` is read as a mix of the key and a solid colour: the candidate (texels of the window
/// further in and clearly not the key, plus black for the outline) whose key-to-colour line passes
/// nearest to it; its alpha is its position on that line, its colour that solid colour plus what
/// the line does not explain (shading kept). Next to a dark solid colour, alpha is at least the
/// `RAMP` of the colour distance to the key, so the black outline stays whole. Deeper texels within
/// `DESPILL` stay opaque, but when their magenta cast (min(R, B) over G) is clearly above that of
/// the solid colours around them (DXT1 blocks mix the key a few texels in), their colour is
/// unmixed the same way. Then the cast of every texel within `DESPILL` is brought down to the usual
/// one of the solid colours around (the crimson hat keeps its own), texels under `ALPHA_MIN` and
/// small groups cut off (`SPECK`) are dropped.
pub fn cut_out(img: &Image, bg: &[bool], key: [u8; 3]) -> Image {
    let (w, h) = (img.w as usize, img.h as usize);
    let key = key.map(f32::from);
    let d = distance_to_background(bg, w, h, DESPILL.ceil().max(BAND.ceil()) as i32);
    let ramp = |p: [f32; 3]| ((dist(p, key) - RAMP[0]) / (RAMP[1] - RAMP[0])).clamp(0.0, 1.0);
    let luma = |c: [f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let cast = |c: [f32; 3]| c[0].min(c[2]) - c[1];
    let mut out = img.clone();
    let mut candidates: Vec<[f32; 3]> = Vec::new();
    let mut casts: Vec<f32> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let n = y * w + x;
            let i = n * 4;
            if bg[n] {
                out.rgba[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            if d[n] > DESPILL {
                continue;
            }
            let p = rgb(img, i);
            let mut c = p;
            let mut alpha = 1.0f32;
            // Solid colours around: further in than this texel (and than the band).
            candidates.clear();
            let deeper = d[n].max(BAND);
            for yy in (y as i32 - CANDIDATE_WINDOW).max(0)..=(y as i32 + CANDIDATE_WINDOW).min(h as i32 - 1) {
                for xx in (x as i32 - CANDIDATE_WINDOW).max(0)..=(x as i32 + CANDIDATE_WINDOW).min(w as i32 - 1) {
                    let m = yy as usize * w + xx as usize;
                    let q = rgb(img, m * 4);
                    if d[m] > deeper && dist(q, key) >= SOLID_MIN {
                        candidates.push(q);
                    }
                }
            }
            // Their usual cast (median: a few DXT1-tinted texels among them do not count).
            casts.clear();
            casts.extend(candidates.iter().map(|&q| cast(q).max(0.0)));
            casts.sort_by(f32::total_cmp);
            let local_cast = casts.get(casts.len() / 2).copied().unwrap_or(0.0);
            let band = d[n] <= BAND;
            if band || cast(p) > local_cast + CAST_MARGIN {
                candidates.push([0.0; 3]);
                // Best line key -> f: smallest distance from p to the segment.
                let (mut best, mut best_t, mut best_f) = (f32::MAX, 0.0f32, [0.0f32; 3]);
                for &f in &candidates {
                    let v = [f[0] - key[0], f[1] - key[1], f[2] - key[2]];
                    let vv = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).max(1.0);
                    let t = (((p[0] - key[0]) * v[0] + (p[1] - key[1]) * v[1] + (p[2] - key[2]) * v[2]) / vv).clamp(0.0, 1.0);
                    let r = dist(p, [key[0] + v[0] * t, key[1] + v[1] * t, key[2] + v[2] * t]);
                    if r < best {
                        (best, best_t, best_f) = (r, t, f);
                    }
                }
                // The solid colour, with the part of p the mix does not explain.
                for k in 0..3 {
                    c[k] = p[k] + (best_f[k] - key[k]) * (1.0 - best_t);
                }
                if band {
                    alpha = best_t;
                    if luma(best_f) < DARK {
                        alpha = alpha.max(ramp(p));
                    }
                    if dist(p, key) < RAMP[0] || alpha < ALPHA_MIN {
                        alpha = 0.0;
                    }
                }
            }
            let m = cast(c) - local_cast;
            if m > 0.0 {
                c[0] -= m;
                c[2] -= m;
            }
            for k in 0..3 {
                out.rgba[i + k] = c[k].round().clamp(0.0, 255.0) as u8;
            }
            out.rgba[i + 3] = (alpha * 255.0).round() as u8;
        }
    }
    drop_specks(&mut out, SPECK);
    out
}

/// Clears (alpha 0) every 8-connected group of visible texels smaller than `min`.
fn drop_specks(img: &mut Image, min: usize) {
    let (w, h) = (img.w as usize, img.h as usize);
    let mut seen = vec![false; w * h];
    let mut group = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if seen[start] || img.rgba[start * 4 + 3] == 0 {
            continue;
        }
        group.clear();
        stack.push(start);
        seen[start] = true;
        while let Some(n) = stack.pop() {
            group.push(n);
            let (x, y) = ((n % w) as i32, (n / w) as i32);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx < 0 || yy < 0 || xx >= w as i32 || yy >= h as i32 {
                        continue;
                    }
                    let m = yy as usize * w + xx as usize;
                    if !seen[m] && img.rgba[m * 4 + 3] != 0 {
                        seen[m] = true;
                        stack.push(m);
                    }
                }
            }
        }
        if group.len() < min {
            for &n in &group {
                img.rgba[n * 4..n * 4 + 4].copy_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
}

/// Rows of the first and last texels of `img` with alpha over half, and the first and last such
/// columns: (left, top, right, bottom), inclusive. None when nothing is that opaque.
pub fn figure_bounds(img: &Image) -> Option<[u32; 4]> {
    let mut b: Option<[u32; 4]> = None;
    for y in 0..img.h {
        for x in 0..img.w {
            if img.rgba[at(img, x, y) + 3] > 127 {
                let r = b.get_or_insert([x, y, x, y]);
                *r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
            }
        }
    }
    b
}

/// Where Jones is drawn (x, y, w, h) next to the background drawn at `background` (x, y, w, h),
/// in the same units: her figure `FIGURE_HEIGHT` background heights tall, its lowest texel row
/// ending exactly on the background's bottom edge, its left edge `FIGURE_OVERLAP` background
/// heights left of the background's right edge. `jones` is the picture's size, `figure` its
/// `figure_bounds`.
pub fn jones_rect(background: [f32; 4], jones: (u32, u32), figure: [u32; 4]) -> [f32; 4] {
    let [bx, by, bw, bh] = background;
    let figure_rows = (figure[3] + 1 - figure[1]) as f32;
    let s = FIGURE_HEIGHT * bh / figure_rows;
    let bottom = by + bh;
    let y = bottom - (figure[3] + 1) as f32 * s;
    let x = bx + bw - FIGURE_OVERLAP * bh - figure[0] as f32 * s;
    [x, y, jones.0 as f32 * s, jones.1 as f32 * s]
}

/// Jones as the page draws her, in screen pixels: the tile rectangle (x, y, w, h) and its texel
/// window (u, v, ul, vl). `picture` is the background's rectangle and `bottom` the pixel edge her
/// feet stand on (just under the last row of the picture's frame). Unlike `jones_rect`, her
/// texel rows end exactly on `bottom` (an integer pixel edge) and the window stops at her last
/// row, so that row is drawn whole and hard; it skips the outermost texel on the other sides
/// (wrapping texture, see the page's padding). `jones` is the padded picture's size.
pub fn jones_tile(picture: [f32; 4], bottom: f32, jones: (u32, u32), figure: [u32; 4]) -> ([f32; 4], [f32; 4]) {
    let [bx, _, bw, bh] = picture;
    let figure_rows = (figure[3] + 1 - figure[1]) as f32;
    let s = FIGURE_HEIGHT * bh / figure_rows;
    let left = bx + bw - FIGURE_OVERLAP * bh;
    let (u0, v0, u1, v1) = (1.0, 1.0, jones.0.saturating_sub(1) as f32, (figure[3] + 1) as f32);
    let x = left - (figure[0] as f32 - u0) * s;
    let y = bottom - (v1 - v0) * s;
    ([x, y, (u1 - u0) * s, (v1 - v0) * s], [u0, v0, u1 - u0, v1 - v0])
}

/// Left-right mirror.
pub fn mirror_h(img: &Image) -> Image {
    let mut out = img.clone();
    for y in 0..img.h {
        for x in 0..img.w {
            let (s, t) = (at(img, x, y), at(img, img.w - 1 - x, y));
            out.rgba[t..t + 4].copy_from_slice(&img.rgba[s..s + 4]);
        }
    }
    out
}

/// The image without its last `rows` rows.
pub fn cut_bottom(img: &Image, rows: u32) -> Image {
    img.rows(0, img.h.saturating_sub(rows))
}

/// Jones, cut out and mirrored, from the four quarters (grid order). None when they are not the
/// expected textures (size, background colour): no character rather than a broken cut-out.
pub fn jones_from_quarters(quarters: &[Image; 4]) -> Option<Image> {
    let mut whole = assemble(quarters)?;
    let corner = rgb(&whole, 0);
    if dist(corner, KEY_EXPECTED.map(f32::from)) > KEY_CHECK {
        return None;
    }
    let key = corner.map(|c| c as u8);
    paint_rects(&mut whole, &ERASE_RECTS, key);
    let bg = flood_background(&whole, key, TOLERANCE, &EXTRA_SEEDS);
    // A sane cut keeps the character: between a quarter and three quarters of the picture.
    let kept = bg.iter().filter(|b| !**b).count() as f32 / bg.len() as f32;
    if !(0.2..0.8).contains(&kept) {
        return None;
    }
    let mut jones = cut_bottom(&mirror_h(&cut_out(&whole, &bg, key)), CUT_BOTTOM);
    retouch(&mut jones, &JONES_KEEP);
    touch_up(&mut jones, &JONES_TOUCH);
    Some(jones)
}

/// Last touches to Jones's cut-out, where the rules above leave a defect on this drawing: the
/// black outline filled or darkened where it is thin or broken (hat, sleeves, shoulder, hands,
/// cheek) and three dots beside it painted over. Texel (x, y) takes the colour given, or is
/// removed.
const JONES_TOUCH: [(u32, u32, Option<[u8; 3]>); 80] = [
    (224, 35, Some([0, 0, 0])),
    (225, 35, Some([0, 0, 0])),
    (226, 35, Some([0, 0, 0])),
    (224, 36, Some([118, 106, 97])),
    (225, 36, Some([106, 95, 86])),
    (226, 36, Some([99, 89, 81])),
    (227, 36, Some([119, 106, 97])),
    (216, 49, Some([0, 0, 0])),
    (217, 49, Some([138, 124, 109])),
    (216, 50, Some([0, 1, 0])),
    (217, 50, Some([112, 99, 88])),
    (215, 62, Some([3, 7, 5])),
    (214, 63, Some([0, 2, 2])),
    (215, 63, Some([69, 62, 54])),
    (215, 64, Some([0, 1, 0])),
    (209, 100, Some([16, 16, 0])),
    (210, 101, Some([16, 16, 0])),
    (208, 102, Some([16, 16, 0])),
    (210, 102, Some([16, 16, 0])),
    (211, 102, Some([106, 67, 35])),
    (208, 103, Some([16, 16, 0])),
    (210, 103, Some([16, 16, 0])),
    (211, 103, Some([106, 67, 35])),
    (208, 104, Some([16, 16, 0])),
    (209, 104, Some([16, 16, 0])),
    (215, 144, Some([109, 66, 33])),
    (215, 145, Some([109, 66, 33])),
    (214, 146, Some([0, 9, 0])),
    (217, 151, Some([107, 65, 33])),
    (219, 152, Some([107, 65, 33])),
    (357, 182, Some([78, 89, 61])),
    (358, 182, Some([67, 76, 53])),
    (359, 182, Some([45, 50, 35])),
    (360, 182, Some([64, 76, 54])),
    (361, 182, Some([69, 80, 58])),
    (368, 192, Some([56, 58, 49])),
    (368, 193, Some([57, 68, 49])),
    (166, 210, Some([60, 69, 51])),
    (166, 211, Some([73, 83, 62])),
    (163, 220, Some([72, 80, 63])),
    (163, 221, Some([93, 103, 81])),
    (377, 224, Some([67, 74, 60])),
    (377, 225, Some([70, 78, 62])),
    (383, 234, Some([41, 28, 11])),
    (385, 234, Some([33, 24, 8])),
    (383, 235, Some([117, 79, 39])),
    (384, 235, Some([45, 29, 16])),
    (385, 235, Some([14, 9, 5])),
    (384, 237, Some([16, 12, 8])),
    (383, 244, Some([17, 11, 5])),
    (384, 244, Some([19, 11, 8])),
    (385, 244, Some([31, 16, 14])),
    (384, 245, Some([63, 40, 18])),
    (385, 245, Some([61, 39, 15])),
    (387, 246, Some([15, 9, 9])),
    (387, 247, Some([15, 9, 9])),
    (388, 247, Some([10, 6, 6])),
    (387, 248, Some([28, 19, 5])),
    (154, 249, Some([48, 55, 43])),
    (386, 249, Some([15, 9, 9])),
    (387, 249, Some([6, 3, 3])),
    (154, 250, Some([54, 62, 49])),
    (386, 250, Some([5, 3, 3])),
    (387, 250, Some([105, 63, 30])),
    (154, 251, Some([68, 78, 62])),
    (386, 251, Some([15, 9, 9])),
    (384, 252, Some([15, 9, 9])),
    (385, 252, Some([15, 9, 9])),
    (386, 252, Some([15, 9, 9])),
    (384, 253, Some([15, 9, 9])),
    (385, 253, Some([15, 9, 9])),
    (384, 254, Some([15, 9, 9])),
    (385, 254, Some([15, 9, 9])),
    (383, 255, Some([15, 9, 9])),
    (382, 257, Some([8, 12, 0])),
    (383, 257, Some([8, 12, 0])),
    (150, 264, Some([43, 50, 34])),
    (150, 265, Some([44, 51, 35])),
    (150, 266, Some([52, 60, 42])),
    (150, 267, Some([69, 82, 57])),
];

/// Applies a list of touches (`JONES_TOUCH`) to a cut-out.
pub fn touch_up(img: &mut Image, touches: &[(u32, u32, Option<[u8; 3]>)]) {
    for &(x, y, colour) in touches {
        if x >= img.w || y >= img.h {
            continue;
        }
        let i = at(img, x, y);
        match colour {
            Some(c) => img.rgba[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]),
            None => img.rgba[i..i + 4].copy_from_slice(&[0, 0, 0, 0]),
        }
    }
}

/// Part of Jones's cut-out (x0, y0, x1, y1) left as the cut made it: the hair under the hat,
/// whose rough edge gives it depth.
const JONES_KEEP: [[u32; 4]; 1] = [[283, 127, 338, 197]];
/// How the edge of Jones's enlarged picture is evened out.
pub(crate) const JONES_EVEN: Even = Even { passes: 40, reach: 0.75, corner: 30.0, keep: &JONES_KEEP };

/// The retouches of a cut-out (`shave_inner`, `defringe`, `clean_edge`), the rectangles `keep` left as they
/// were.
pub fn retouch(img: &mut Image, keep: &[[u32; 4]]) {
    let before = img.clone();
    shave_inner(img);
    defringe(img);
    clean_edge(img);
    for z in keep {
        for y in z[1]..z[3].min(img.h) {
            for x in z[0]..z[2].min(img.w) {
                let i = at(img, x, y);
                img.rgba[i..i + 4].copy_from_slice(&before.rgba[i..i + 4]);
            }
        }
    }
}

/// How far inside the edge the background's tint is looked for, in texels.
const FRINGE_DEPTH: u8 = 2;
/// Texels around a tinted one in which its clean colour is looked for.
const FRINGE_WINDOW: i32 = 3;
/// Cast (min(R, B) - G) above the usual one of the clean colours around from which a texel is
/// tinted, and under which a clean colour may replace it.
const FRINGE_CAST: [f32; 2] = [8.0, 4.0];
/// Cast from which a texel may hold the background's tint at all (green not above red and blue):
/// a texel that is only a darker shade of the drawing is never repainted.
const FRINGE_PINK: f32 = -6.0;
/// Usual thickness of the outline, in texels: a tinted texel deeper than that is never outline.
const OUTLINE_THICK: usize = 2;
/// Share of the clean colours' luma under which a tinted texel belongs to the outline.
const FRINGE_INK: f32 = 0.6;

/// Repaints the texels near the edge that kept a tint of the removed background (pinkish dots
/// along a cheek, at the corner of the lips): a texel within `FRINGE_DEPTH` of the black
/// outline or of the outside whose cast is clearly above that of the drawing's colours a
/// little further in takes the colour of the nearest of those, or the outline's when it is
/// much darker than them (a tinted piece of the outline). Nothing else changes: not the shape,
/// not a texel that is only darker than its neighbours.
pub fn defringe(img: &mut Image) {
    let (w, h) = (img.w as i32, img.h as i32);
    let n = (w * h) as usize;
    let at = |x: i32, y: i32| (y * w + x) as usize;
    let inside = |x: i32, y: i32| (0..w).contains(&x) && (0..h).contains(&y);
    const SIDES: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let before = img.rgba.clone();
    let rgb = |i: usize| [before[i * 4] as f32, before[i * 4 + 1] as f32, before[i * 4 + 2] as f32];
    let luma = |i: usize| {
        let c = rgb(i);
        0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
    };
    let cast = |i: usize| {
        let c = rgb(i);
        c[0].min(c[2]) - c[1]
    };
    let solid: Vec<bool> = before.chunks_exact(4).map(|p| p[3] > 127).collect();
    let dark: Vec<bool> = (0..n).map(|i| solid[i] && luma(i) < OUTLINE_LUMA as f32).collect();
    let light: Vec<bool> = (0..n).map(|i| solid[i] && !dark[i]).collect();
    // The edge: the outside, and the dark texels near it (the outline).
    let edge = |x: i32, y: i32| !solid[at(x, y)] || (dark[at(x, y)] && (-3..=3).any(|dy| (-3..=3).any(|dx| inside(x + dx, y + dy) && !solid[at(x + dx, y + dy)])));
    // Depth of the light texels behind it (0: not reached).
    let mut depth = vec![0u8; n];
    let mut front: Vec<usize> = (0..n).filter(|&i| edge(i as i32 % w, i as i32 / w)).collect();
    for d in 1..=FRINGE_DEPTH + 1 {
        let mut next = Vec::new();
        for &i in &front {
            let (x, y) = (i as i32 % w, i as i32 / w);
            for (dx, dy) in SIDES {
                if inside(x + dx, y + dy) {
                    let m = at(x + dx, y + dy);
                    if light[m] && depth[m] == 0 {
                        depth[m] = d;
                        next.push(m);
                    }
                }
            }
        }
        front = next;
    }
    // Small closed groups of light texels are strays outside the outline (`clean_edge`).
    let mut group_size = vec![0usize; n];
    let mut seen = vec![false; n];
    let (mut group, mut stack) = (Vec::new(), Vec::new());
    for start in 0..n {
        if !light[start] || seen[start] {
            continue;
        }
        group.clear();
        stack.push(start);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            group.push(i);
            let (x, y) = (i as i32 % w, i as i32 / w);
            for (dx, dy) in SIDES {
                if inside(x + dx, y + dy) {
                    let m = at(x + dx, y + dy);
                    if light[m] && !seen[m] {
                        seen[m] = true;
                        stack.push(m);
                    }
                }
            }
        }
        for &i in &group {
            group_size[i] = group.len();
        }
    }
    let clean = |i: usize| light[i] && (depth[i] == 0 || depth[i] > FRINGE_DEPTH);
    // Solid texels within the outline's usual thickness of the outside.
    let mut rim = vec![false; n];
    let mut front: Vec<usize> = (0..n).filter(|&i| !solid[i]).collect();
    for _ in 0..OUTLINE_THICK {
        let mut next = Vec::new();
        for &i in &front {
            let (x, y) = (i as i32 % w, i as i32 / w);
            for (dx, dy) in SIDES {
                if inside(x + dx, y + dy) {
                    let m = at(x + dx, y + dy);
                    if solid[m] && !rim[m] {
                        rim[m] = true;
                        next.push(m);
                    }
                }
            }
        }
        front = next;
    }
    let mut around: Vec<(i32, usize)> = Vec::new();
    let mut values: Vec<f32> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let i = at(x, y);
            if depth[i] == 0 || depth[i] > FRINGE_DEPTH || group_size[i] <= STRAY_MAX {
                continue;
            }
            around.clear();
            let mut ink: Option<(i32, usize)> = None;
            for dy in -FRINGE_WINDOW..=FRINGE_WINDOW {
                for dx in -FRINGE_WINDOW..=FRINGE_WINDOW {
                    if !inside(x + dx, y + dy) {
                        continue;
                    }
                    let (d, m) = (dx * dx + dy * dy, at(x + dx, y + dy));
                    if clean(m) {
                        around.push((d, m));
                    } else if dark[m] && ink.map_or(true, |(best, _)| d < best) {
                        ink = Some((d, m));
                    }
                }
            }
            if around.len() < 4 {
                continue;
            }
            let mut median = |of: &dyn Fn(usize) -> f32| {
                values.clear();
                values.extend(around.iter().map(|&(_, m)| of(m)));
                values.sort_by(f32::total_cmp);
                values[values.len() / 2]
            };
            let usual = median(&cast);
            if cast(i) < FRINGE_PINK || cast(i) <= usual + FRINGE_CAST[0] {
                continue;
            }
            if rim[i] && luma(i) < FRINGE_INK * median(&luma) {
                // A tinted piece of the outline.
                if let Some((_, m)) = ink {
                    img.rgba[i * 4..i * 4 + 3].copy_from_slice(&before[m * 4..m * 4 + 3]);
                }
                continue;
            }
            let good = |m: usize| cast(m) <= usual + FRINGE_CAST[1];
            let Some(nearest) = around.iter().filter(|&&(_, m)| good(m)).map(|&(d, _)| d).min() else { continue };
            let (mut sum, mut count) = ([0.0f32; 3], 0.0f32);
            for &(d, m) in &around {
                if d <= nearest + 2 && good(m) {
                    let c = rgb(m);
                    sum = [sum[0] + c[0], sum[1] + c[1], sum[2] + c[2]];
                    count += 1.0;
                }
            }
            for ch in 0..3 {
                img.rgba[i * 4 + ch] = (sum[ch] / count).round() as u8;
            }
        }
    }
}

/// Paints over the small growths of the black outline into the drawing (a black dot on a
/// cheek): dark texels, at most `BUMP_MAX` in a row, that stick out of the outline towards the
/// inside with the drawing's colour on their three other sides, where the outline stays two
/// texels thick behind them. They take the colour of the drawing in front of them.
pub fn shave_inner(img: &mut Image) {
    let (w, h) = (img.w as i32, img.h as i32);
    let n = (w * h) as usize;
    let at = |x: i32, y: i32| (y * w + x) as usize;
    let inside = |x: i32, y: i32| (0..w).contains(&x) && (0..h).contains(&y);
    let before = img.rgba.clone();
    let solid: Vec<bool> = before.chunks_exact(4).map(|p| p[3] > 127).collect();
    let dark: Vec<bool> = before.chunks_exact(4).map(|p| p[3] > 127 && (299 * p[0] as u32 + 587 * p[1] as u32 + 114 * p[2] as u32) / 1000 < OUTLINE_LUMA).collect();
    let light: Vec<bool> = (0..n).map(|i| solid[i] && !dark[i]).collect();
    let outline: Vec<bool> = (0..n)
        .map(|i| {
            let (x, y) = (i as i32 % w, i as i32 / w);
            dark[i] && (-3..=3).any(|dy| (-3..=3).any(|dx| inside(x + dx, y + dy) && !solid[at(x + dx, y + dy)]))
        })
        .collect();
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        // Along the edge: across the way in.
        let (px, py) = (dy, dx);
        let growth = |x: i32, y: i32| {
            (3..w - 3).contains(&x)
                && (3..h - 3).contains(&y)
                && dark[at(x, y)]
                && light[at(x + dx, y + dy)]
                && dark[at(x - dx, y - dy)]
                && dark[at(x - 2 * dx, y - 2 * dy)]
                && (outline[at(x - dx, y - dy)] || outline[at(x - 2 * dx, y - 2 * dy)])
        };
        for y in 3..h - 3 {
            for x in 3..w - 3 {
                if !growth(x, y) || growth(x - px, y - py) {
                    continue;
                }
                let mut len = 1;
                while growth(x + px * len, y + py * len) {
                    len += 1;
                }
                if len > BUMP_MAX || !light[at(x - px, y - py)] || !light[at(x + px * len, y + py * len)] {
                    continue;
                }
                for k in 0..len {
                    let (rx, ry) = (x + px * k, y + py * k);
                    let from = if light[at(rx + 2 * dx, ry + 2 * dy)] { at(rx + 2 * dx, ry + 2 * dy) } else { at(rx + dx, ry + dy) };
                    img.rgba[at(rx, ry) * 4..at(rx, ry) * 4 + 3].copy_from_slice(&before[from * 4..from * 4 + 3]);
                }
            }
        }
    }
}

/// Luma from which a texel is not the black outline.
const OUTLINE_LUMA: u32 = 20;
/// Largest group of coloured texels taken for a stray outside the outline.
const STRAY_MAX: usize = 16;
/// Longest bump shaved off the outline, in texels.
const BUMP_MAX: i32 = 3;

/// Cleans the edge of a cut-out character drawn with a black outline, without changing its
/// shape more than a retoucher would:
/// - coloured texels left outside the outline (the texture's blocks mix the background with
///   the outline, and a few texels of the mix read as a colour of their own: red dots over the
///   hat) take the colour of the outline they touch; those further out go. They are the small
///   groups of texels that are not dark, touch the outside and are closed off from the rest of
///   the drawing by the outline (4-connected, so that a gap in the outline does not make a
///   stray of the skin behind it);
/// - bumps of the outline go: dark, at most `BUMP_MAX` texels long, one texel high, where the
///   outline stays two texels thick behind them;
/// - faint texels left with nothing solid beside them, and small groups left on their own, go.
pub fn clean_edge(img: &mut Image) {
    let (w, h) = (img.w as i32, img.h as i32);
    let n = (w * h) as usize;
    let at = |x: i32, y: i32| (y * w + x) as usize;
    let inside = |x: i32, y: i32| (0..w).contains(&x) && (0..h).contains(&y);
    const SIDES: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let luma = |p: &[u8]| (299 * p[0] as u32 + 587 * p[1] as u32 + 114 * p[2] as u32) / 1000;
    // Strays.
    let light: Vec<bool> = img.rgba.chunks_exact(4).map(|p| p[3] > 0 && luma(p) >= OUTLINE_LUMA).collect();
    let faint: Vec<bool> = img.rgba.chunks_exact(4).map(|p| p[3] < 128).collect();
    let outline: Vec<bool> = (0..n).map(|i| !faint[i] && !light[i]).collect();
    let before = img.rgba.clone();
    let mut seen = vec![false; n];
    let (mut group, mut stack) = (Vec::new(), Vec::new());
    for start in 0..n {
        if !light[start] || seen[start] {
            continue;
        }
        group.clear();
        stack.push(start);
        seen[start] = true;
        let mut outside = false;
        while let Some(i) = stack.pop() {
            group.push(i);
            outside |= faint[i];
            let (x, y) = (i as i32 % w, i as i32 / w);
            for (dx, dy) in SIDES {
                if !inside(x + dx, y + dy) {
                    continue;
                }
                let m = at(x + dx, y + dy);
                outside |= faint[m];
                if light[m] && !seen[m] {
                    seen[m] = true;
                    stack.push(m);
                }
            }
        }
        if !outside || group.len() > STRAY_MAX {
            continue;
        }
        for &i in &group {
            let (x, y) = (i as i32 % w, i as i32 / w);
            let beside = SIDES.iter().map(|&(dx, dy)| (x + dx, y + dy)).find(|&(xx, yy)| inside(xx, yy) && outline[at(xx, yy)]);
            match beside {
                Some((xx, yy)) => {
                    let m = at(xx, yy) * 4;
                    img.rgba[i * 4..i * 4 + 3].copy_from_slice(&before[m..m + 3]);
                }
                None => img.rgba[i * 4..i * 4 + 4].copy_from_slice(&[0, 0, 0, 0]),
            }
        }
    }
    // Bumps of the outline, in the four directions.
    let solid: Vec<bool> = img.rgba.chunks_exact(4).map(|p| p[3] > 127).collect();
    let dark: Vec<bool> = img.rgba.chunks_exact(4).map(|p| p[3] > 127 && luma(p) < OUTLINE_LUMA).collect();
    let mut drop = Vec::new();
    for dir in 0..4 {
        let (lw, lh) = if dir < 2 { (w, h) } else { (h, w) };
        // (u, v): along the edge, towards the inside.
        let texel = |u: i32, v: i32| match dir {
            0 => at(u, v),
            1 => at(u, h - 1 - v),
            2 => at(v, u),
            _ => at(w - 1 - v, u),
        };
        let s = |u: i32, v: i32| solid[texel(u, v)];
        for v in 1..lh - 2 {
            let mut u = 1;
            while u < lw - 1 {
                if !(s(u, v) && !s(u, v - 1) && !s(u - 1, v)) {
                    u += 1;
                    continue;
                }
                let mut end = u;
                while end + 1 < lw - 1 && s(end + 1, v) && !s(end + 1, v - 1) {
                    end += 1;
                }
                let bump = end - u < BUMP_MAX && !s(end + 1, v) && !s(u - 1, v - 1) && !s(end + 1, v - 1) && (u - 1..=end + 1).all(|k| s(k, v + 1));
                if bump && (u..=end).all(|k| (0..3).all(|d| dark[texel(k, v + d)])) {
                    drop.extend((u..=end).map(|k| texel(k, v)));
                }
                u = end + 1;
            }
        }
    }
    for i in drop {
        img.rgba[i * 4..i * 4 + 4].copy_from_slice(&[0, 0, 0, 0]);
    }
    // Faint texels with nothing solid beside them.
    let solid: Vec<bool> = img.rgba.chunks_exact(4).map(|p| p[3] > 127).collect();
    for y in 0..h {
        for x in 0..w {
            let i = at(x, y);
            if img.rgba[i * 4 + 3] > 0 && !solid[i] && !SIDES.iter().any(|&(dx, dy)| inside(x + dx, y + dy) && solid[at(x + dx, y + dy)]) {
                img.rgba[i * 4..i * 4 + 4].copy_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    drop_specks(img, SPECK);
}

/// Sub-rows per output row when the silhouette is filled.
const FILL_ROWS: u32 = 4;

/// Closed loops of the boundary between the texels with alpha over half and the others, on the
/// lattice of texel corners (holes included; any decomposition fills the same with the even-odd
/// rule).
pub fn trace_silhouette(img: &Image) -> Vec<Vec<[f32; 2]>> {
    let (w, h) = (img.w as i32, img.h as i32);
    let inside = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && img.rgba[at(img, x as u32, y as u32) + 3] > 127;
    let vw = (w + 1) as usize;
    let none = u32::MAX;
    // Up to two edges leave a corner (two texels touching by it).
    let mut next = vec![[none; 2]; vw * (h + 1) as usize];
    let mut add = |from: (i32, i32), to: (i32, i32)| {
        let slot = &mut next[from.1 as usize * vw + from.0 as usize];
        let v = (to.1 as usize * vw + to.0 as usize) as u32;
        if slot[0] == none {
            slot[0] = v;
        } else {
            slot[1] = v;
        }
    };
    for y in 0..h {
        for x in 0..w {
            if !inside(x, y) {
                continue;
            }
            if !inside(x, y - 1) {
                add((x, y), (x + 1, y));
            }
            if !inside(x + 1, y) {
                add((x + 1, y), (x + 1, y + 1));
            }
            if !inside(x, y + 1) {
                add((x + 1, y + 1), (x, y + 1));
            }
            if !inside(x - 1, y) {
                add((x, y + 1), (x, y));
            }
        }
    }
    let mut loops = Vec::new();
    for start in 0..next.len() {
        while next[start][0] != none || next[start][1] != none {
            let mut points = Vec::new();
            let mut v = start;
            loop {
                points.push([(v % vw) as f32, (v / vw) as f32]);
                let slot = &mut next[v];
                let k = if slot[0] != none { 0 } else { 1 };
                let to = slot[k];
                slot[k] = none;
                if to == none {
                    break;
                }
                v = to as usize;
                if v == start {
                    break;
                }
            }
            if points.len() >= 4 {
                loops.push(points);
            }
        }
    }
    loops
}

/// A closed loop of unit steps along the texels' edges (`trace_silhouette`) with its staircases
/// turned into slopes, without leaving the drawing's shape: the loop is cut into straight runs;
/// next to a run one texel long (the step of a staircase) a run is cut at its middle, so that a
/// staircase of any slope becomes one straight line; elsewhere a corner only loses half a texel.
/// No point of the result is further than half a texel from the loop.
pub fn smooth_loop(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let n = points.len();
    if n < 4 {
        return points.to_vec();
    }
    // Corners: the points where the direction changes.
    let dir = |i: usize| {
        let (a, b) = (points[i % n], points[(i + 1) % n]);
        [b[0] - a[0], b[1] - a[1]]
    };
    let corners: Vec<usize> = (0..n).filter(|&i| dir(i) != dir(i + n - 1)).collect();
    let m = corners.len();
    if m < 3 {
        return points.to_vec();
    }
    // Runs between corners: start, end, length.
    let run = |k: usize| {
        let (a, b) = (points[corners[k % m]], points[corners[(k + 1) % m]]);
        (a, b, (b[0] - a[0]).abs() + (b[1] - a[1]).abs())
    };
    let mut out = Vec::with_capacity(2 * m);
    for k in 0..m {
        let (a, b, len) = run(k);
        if len <= 0.0 {
            continue;
        }
        let (before, after) = (run(k + m - 1).2, run(k + 1).2);
        let cut = |neighbour: f32| if len <= 1.0 || neighbour <= 1.0 { len * 0.5 } else { (len * 0.5).min(0.5) };
        let (s, e) = (cut(before), cut(after));
        let at = |t: f32| [a[0] + (b[0] - a[0]) * t / len, a[1] + (b[1] - a[1]) * t / len];
        out.push(at(s));
        if s + e < len {
            out.push(at(len - e));
        }
    }
    out
}

/// How a smoothed loop is evened out (`even_loop`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Even {
    /// Smoothing passes over points half a texel apart.
    pub passes: u32,
    /// Furthest a point may move, in texels.
    pub reach: f32,
    /// Turn, in degrees over `EVEN_SPAN` texels each side, from which a point is a corner of the
    /// drawing and stays where it is.
    pub corner: f32,
    /// Rectangles (x0, y0, x1, y1, in texels) whose points stay where they are.
    pub keep: &'static [[u32; 4]],
}

/// Texels each side of a point over which its turn is measured.
const EVEN_SPAN: f32 = 3.0;

/// A closed loop (`smooth_loop`) evened out as a pen stroke: small bumps and hollows along its
/// straight and gently curved stretches are levelled, no point moving further than `reach`
/// texels; the corners of the drawing stay where they are. A point never moves into the
/// drawing (`solid`: true inside it): hollows are filled, nothing is cut off, so that an
/// outline one texel thick stays whole.
pub fn even_loop(points: &[[f32; 2]], even: Even, solid: impl Fn(f32, f32) -> bool) -> Vec<[f32; 2]> {
    const STEP: f32 = 0.5;
    if points.len() < 3 {
        return points.to_vec();
    }
    // Points at most STEP apart.
    let mut base: Vec<[f32; 2]> = Vec::new();
    for i in 0..points.len() {
        let (a, b) = (points[i], points[(i + 1) % points.len()]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let parts = (len / STEP).ceil().max(1.0) as usize;
        for j in 0..parts {
            let t = j as f32 / parts as f32;
            base.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    let n = base.len();
    let span = ((EVEN_SPAN / STEP) as usize).min(n / 3);
    if span == 0 {
        return points.to_vec();
    }
    let limit = even.corner.to_radians().dcos();
    let pinned: Vec<bool> = (0..n)
        .map(|i| {
            let (a, b, c) = (base[(i + n - span) % n], base[i], base[(i + span) % n]);
            let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
            let (lu, lv) = ((u[0] * u[0] + u[1] * u[1]).sqrt(), (v[0] * v[0] + v[1] * v[1]).sqrt());
            let kept = even.keep.iter().any(|z| b[0] >= z[0] as f32 && b[0] <= z[2] as f32 && b[1] >= z[1] as f32 && b[1] <= z[3] as f32);
            kept || (lu > 0.0 && lv > 0.0 && (u[0] * v[0] + u[1] * v[1]) / (lu * lv) < limit)
        })
        .collect();
    let mut now = base.clone();
    let mut next = base.clone();
    for _ in 0..even.passes {
        for i in 0..n {
            if pinned[i] {
                continue;
            }
            let (a, b, c) = (now[(i + n - 1) % n], now[i], now[(i + 1) % n]);
            let mut q = [(a[0] + 2.0 * b[0] + c[0]) * 0.25, (a[1] + 2.0 * b[1] + c[1]) * 0.25];
            let d = [q[0] - base[i][0], q[1] - base[i][1]];
            let far = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if far > even.reach {
                q = [base[i][0] + d[0] * even.reach / far, base[i][1] + d[1] * even.reach / far];
            }
            next[i] = q;
        }
        std::mem::swap(&mut now, &mut next);
        next.copy_from_slice(&now);
    }
    for i in 0..n {
        let d = [now[i][0] - base[i][0], now[i][1] - base[i][1]];
        let far = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if far > 0.0 && solid(base[i][0] + d[0] * 0.5 / far, base[i][1] + d[1] * 0.5 / far) {
            now[i] = base[i];
        }
    }
    now
}

/// Rows of `out` (each `stride` long) filled on up to 4 threads, each row by `f(row, line)`.
fn par_lines<T: Send>(out: &mut [T], stride: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let rows = out.len() / stride.max(1);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(4).min(rows.max(1));
    if threads <= 1 {
        for (y, line) in out.chunks_mut(stride).enumerate() {
            f(y, line);
        }
        return;
    }
    let per = rows.div_ceil(threads);
    let f = &f;
    let priority = openxiii_package::background::current();
    std::thread::scope(|sc| {
        for (n, part) in out.chunks_mut(per * stride).enumerate() {
            let math = openxiii_math::Mode::current();
            sc.spawn(move || {
                let _math = math.enter();
                openxiii_package::background::take(priority);
                for (y, line) in part.chunks_mut(stride).enumerate() {
                    f(n * per + y, line);
                }
            });
        }
    });
}

/// Coverage (0 to 1) of each texel of a `w` × `h` picture by the loops (even-odd rule), their
/// coordinates multiplied by `k`.
pub fn fill_loops(loops: &[Vec<[f32; 2]>], k: f32, w: u32, h: u32) -> Vec<f32> {
    let mut out = vec![0.0f32; (w * h) as usize];
    let mut edges: Vec<[f32; 4]> = Vec::new();
    for l in loops {
        for i in 0..l.len() {
            let (a, b) = (l[i], l[(i + 1) % l.len()]);
            if a[1] != b[1] {
                edges.push([a[0] * k, a[1] * k, b[0] * k, b[1] * k]);
            }
        }
    }
    let share = 1.0 / FILL_ROWS as f32;
    let mut xs: Vec<f32> = Vec::new();
    // The edges by their top: the sample lines go down, so the edges a line can cross are the
    // ones started and not ended yet (the same edges as a scan of all of them finds).
    edges.sort_by(|a, b| a[1].min(a[3]).total_cmp(&b[1].min(b[3])));
    let mut next = 0usize;
    let mut active: Vec<[f32; 4]> = Vec::new();
    for row in 0..h {
        let line = &mut out[(row * w) as usize..((row + 1) * w) as usize];
        for s in 0..FILL_ROWS {
            let y = row as f32 + (s as f32 + 0.5) * share;
            xs.clear();
            while next < edges.len() && edges[next][1].min(edges[next][3]) <= y {
                active.push(edges[next]);
                next += 1;
            }
            active.retain(|e| y < e[1].max(e[3]));
            for e in &active {
                let (y0, y1) = (e[1].min(e[3]), e[1].max(e[3]));
                if y >= y0 && y < y1 {
                    xs.push(e[0] + (e[2] - e[0]) * (y - e[1]) / (e[3] - e[1]));
                }
            }
            xs.sort_by(f32::total_cmp);
            for pair in xs.chunks_exact(2) {
                let (x0, x1) = (pair[0].clamp(0.0, w as f32), pair[1].clamp(0.0, w as f32));
                let (first, last) = (x0.floor() as usize, (x1.ceil() as usize).min(w as usize));
                for (x, v) in line.iter_mut().enumerate().take(last).skip(first) {
                    let cover = (x1.min(x as f32 + 1.0) - x0.max(x as f32)).max(0.0);
                    *v += cover * share;
                }
            }
        }
    }
    out
}

/// Catmull-Rom weights of the four texels around a position `t` past the second one.
fn cubic(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [(-t3 + 2.0 * t2 - t) * 0.5, (3.0 * t3 - 5.0 * t2 + 2.0) * 0.5, (-3.0 * t3 + 4.0 * t2 + t) * 0.5, (t3 - t2) * 0.5]
}

/// The cut-out `img` at `scale` / SCALE_ONE times its size with its edge anti-aliased and nothing
/// else changed: the silhouette keeps the drawing's shape (`smooth_loop`: staircases become
/// slopes, never more than half a texel away), filled with the coverage of each pixel; the colours
/// are the drawing's own, enlarged (bicubic, carried outwards first so the edge takes the
/// drawing's colour and not the background's). No outline is added, nothing is blurred.
///
/// `plain`: the simple variant, without the slopes: the mask itself, slightly softened, enlarged
/// and tightened to an edge about a pixel and a half wide (the steps stay, with soft corners).
///
/// `soft_zones`: rectangles of the picture (x0, y0, x1, y1, in texels) where the simple variant
/// is used instead of the slopes (a head, a bun: small rounded shapes read better with their
/// steps only softened), with a few texels of transition around them.
pub fn smooth_edge(img: &Image, scale: u32, plain: bool, soft_zones: &[[u32; 4]]) -> Image {
    smooth_edge_with(img, scale, plain, soft_zones, None)
}

/// `smooth_edge`, the silhouette evened out as a pen stroke when `even` is given (`even_loop`).
pub fn smooth_edge_with(img: &Image, scale: u32, plain: bool, soft_zones: &[[u32; 4]], even: Option<Even>) -> Image {
    let (w, h) = (img.w as usize, img.h as usize);
    let k = scale.max(1) as f32 / SCALE_ONE as f32;
    let (ow, oh) = (((w as f32 * k).round() as usize).max(1), ((h as f32 * k).round() as usize).max(1));
    let mut out = Image::new(ow as u32, oh as u32);
    if w == 0 || h == 0 {
        return out;
    }
    let loops: Vec<Vec<[f32; 2]>> = trace_silhouette(img)
        .iter()
        .map(|l| smooth_loop(l))
        .filter(|l| l.len() >= 3)
        .map(|l| match even {
            Some(e) => even_loop(&l, e, |x, y| {
                x >= 0.0 && y >= 0.0 && (x as usize) < w && (y as usize) < h && img.rgba[((y as usize) * w + x as usize) * 4 + 3] > 127
            }),
            None => l,
        })
        .collect();
    let mut shape = if plain { plain_edge(img, k, ow, oh) } else { fill_loops(&loops, k, ow as u32, oh as u32) };
    if !plain && !soft_zones.is_empty() {
        // The pixels a zone can reach (the zone and its transition, with a margin): only those
        // are looked at; everywhere else the weight is 0 and nothing changes.
        let reach = ZONE_FEATHER + 1.0;
        let boxes: Vec<[usize; 4]> = soft_zones
            .iter()
            .map(|z| {
                let lo = |v: u32| (((v as f32 - reach) * k).floor().max(0.0)) as usize;
                let hi = |v: u32, n: usize| ((((v as f32 + reach) * k).ceil().max(0.0)) as usize + 1).min(n);
                [lo(z[0]), lo(z[1]), hi(z[2], ow), hi(z[3], oh)]
            })
            .collect();
        let simple = plain_edge_in(img, k, ow, oh, &boxes);
        for y in 0..oh {
            if !boxes.iter().any(|b| y >= b[1] && y < b[3]) {
                continue;
            }
            let sy = (y as f32 + 0.5) / k;
            for x in 0..ow {
                if !boxes.iter().any(|b| y >= b[1] && y < b[3] && x >= b[0] && x < b[2]) {
                    continue;
                }
                let sx = (x as f32 + 0.5) / k;
                let weight = soft_zones.iter().fold(0.0f32, |best, z| {
                    let dx = (z[0] as f32 - sx).max(sx - z[2] as f32).max(0.0);
                    let dy = (z[1] as f32 - sy).max(sy - z[3] as f32).max(0.0);
                    best.max(1.0 - (dx * dx + dy * dy).sqrt() / ZONE_FEATHER)
                });
                if weight > 0.0 {
                    let n = y * ow + x;
                    shape[n] = shape[n] * (1.0 - weight) + simple[n] * weight;
                }
            }
        }
    }
    // Colours of the solid texels carried outwards, a ring at a time.
    let mut colour: Vec<[f32; 3]> = img.rgba.chunks_exact(4).map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
    let mut known: Vec<bool> = img.rgba.chunks_exact(4).map(|p| p[3] > 127).collect();
    for _ in 0..3 {
        let mut fresh = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if known[y * w + x] {
                    continue;
                }
                let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
                for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                    for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                        if known[yy * w + xx] {
                            let c = colour[yy * w + xx];
                            sum = [sum[0] + c[0], sum[1] + c[1], sum[2] + c[2]];
                            n += 1.0;
                        }
                    }
                }
                if n > 0.0 {
                    fresh.push((y * w + x, [sum[0] / n, sum[1] / n, sum[2] / n]));
                }
            }
        }
        if fresh.is_empty() {
            break;
        }
        for (i, c) in fresh {
            colour[i] = c;
            known[i] = true;
        }
    }
    // Enlarged across, then down; only where the texel was solid or just carried (elsewhere the
    // nearest known colour: the bicubic never reads the background).
    let at = |x: isize, y: isize| {
        let i = y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize;
        known[i].then(|| colour[i])
    };
    // Per column: first texel, weights and nearest texel (the same values as computed per pixel).
    let cols: Vec<(isize, [f32; 4], isize)> = (0..ow)
        .map(|x| {
            let fx = (x as f32 + 0.5) / k - 0.5;
            (fx.floor() as isize, cubic(fx - fx.floor()), fx.round() as isize)
        })
        .collect();
    let shape = &shape;
    par_lines(&mut out.rgba, ow * 4, |y, line| {
        let fy = (y as f32 + 0.5) / k - 0.5;
        let (by, wy) = (fy.floor() as isize, cubic(fy - fy.floor()));
        for x in 0..ow {
            let n = y * ow + x;
            let alpha = shape[n].clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let (bx, wx, rx) = cols[x];
            // The nearest texel's colour stands for the texels the carrying did not reach.
            let near = at(rx, fy.round() as isize).unwrap_or([0.0; 3]);
            let mut c = [0.0f32; 3];
            for (oy, wty) in wy.iter().enumerate() {
                for (ox, wtx) in wx.iter().enumerate() {
                    let s = at(bx + ox as isize - 1, by + oy as isize - 1).unwrap_or(near);
                    let wt = wtx * wty;
                    c = [c[0] + s[0] * wt, c[1] + s[1] * wt, c[2] + s[2] * wt];
                }
            }
            let i = x * 4;
            for ch in 0..3 {
                line[i + ch] = c[ch].round().clamp(0.0, 255.0) as u8;
            }
            line[i + 3] = (alpha * 255.0).round() as u8;
        }
    });
    out
}

/// Texels over which a soft zone of `smooth_edge` fades into the rest.
const ZONE_FEATHER: f32 = 4.0;

/// Coverage of the simple variant of `smooth_edge`: the mask (alpha over half) softened over its
/// neighbours (1 2 1 across and down), read between texels (bilinear) at the new size, then
/// tightened around its middle value so that the edge is about a pixel and a half wide.
fn plain_edge(img: &Image, k: f32, ow: usize, oh: usize) -> Vec<f32> {
    plain_edge_in(img, k, ow, oh, &[[0, 0, ow, oh]])
}

/// `plain_edge` for the pixels inside `boxes` (x0, y0, x1, y1) only; 0 elsewhere.
fn plain_edge_in(img: &Image, k: f32, ow: usize, oh: usize, boxes: &[[usize; 4]]) -> Vec<f32> {
    let (w, h) = (img.w as usize, img.h as usize);
    let solid = |x: isize, y: isize| {
        let (x, y) = (x.clamp(0, w as isize - 1) as usize, y.clamp(0, h as isize - 1) as usize);
        if img.rgba[(y * w + x) * 4 + 3] > 127 { 1.0f32 } else { 0.0 }
    };
    let mut soft = vec![0.0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let mut sum = 0.0;
            for (dy, wy) in [(-1, 1.0), (0, 2.0), (1, 1.0)] {
                for (dx, wx) in [(-1, 1.0), (0, 2.0), (1, 1.0)] {
                    sum += solid(x + dx, y + dy) * wx * wy;
                }
            }
            soft[y as usize * w + x as usize] = sum / 16.0;
        }
    }
    let at = |x: isize, y: isize| soft[y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize];
    let gain = (k * 1.33).max(1.0);
    let mut out = vec![0.0f32; ow * oh];
    par_lines(&mut out, ow, |y, line| {
        if !boxes.iter().any(|b| y >= b[1] && y < b[3]) {
            return;
        }
        let fy = (y as f32 + 0.5) / k - 0.5;
        let (by, ty) = (fy.floor() as isize, fy - fy.floor());
        for (x, o) in line.iter_mut().enumerate() {
            if !boxes.iter().any(|b| y >= b[1] && y < b[3] && x >= b[0] && x < b[2]) {
                continue;
            }
            let fx = (x as f32 + 0.5) / k - 0.5;
            let (bx, tx) = (fx.floor() as isize, fx - fx.floor());
            let v = at(bx, by) * (1.0 - tx) * (1.0 - ty) + at(bx + 1, by) * tx * (1.0 - ty) + at(bx, by + 1) * (1.0 - tx) * ty + at(bx + 1, by + 1) * tx * ty;
            *o = ((v - 0.5) * gain + 0.5).clamp(0.0, 1.0);
        }
    });
    out
}

/// The background window of the video frame (None when the frame is not 640 × 480).
pub fn background_from_frame(frame: &Image) -> Option<Image> {
    if (frame.w, frame.h) != BACKGROUND_FRAME_SIZE || frame.rgba.len() != (frame.w * frame.h * 4) as usize {
        return None;
    }
    let [x, y, w, h] = BACKGROUND_CROP;
    let mut out = frame.region(x, y, w, h);
    out.rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    Some(out)
}

/// The video file of the background.
pub fn background_video(video_dir: &Path) -> std::path::PathBuf {
    video_dir.join(format!("{BACKGROUND_VIDEO}.bik"))
}

/// Background picture from the player's Video folder when its frame is ready (still_frames.rs;
/// None while it is being made, or without the video or ffmpeg). Never blocks.
pub fn background(video_dir: &Path) -> Option<Image> {
    let f = crate::still_frames::get(&background_video(video_dir), BACKGROUND_FRAME)?;
    background_from_frame(&Image { w: f.w, h: f.h, rgba: f.rgba.clone() })
}

/// True once the background's frame is made (found or not): nothing more to wait for.
pub fn background_settled(video_dir: &Path) -> bool {
    crate::still_frames::is_done(&background_video(video_dir), BACKGROUND_FRAME)
}

/// Jones from the game's textures (`texture` reads the first level of a texture by name).
pub fn jones_from_textures(mut texture: impl FnMut(&str) -> Option<Image>) -> Option<Image> {
    let quarters = [texture(JONES_TEXTURES[0])?, texture(JONES_TEXTURES[1])?, texture(JONES_TEXTURES[2])?, texture(JONES_TEXTURES[3])?];
    jones_from_quarters(&quarters)
}

/// Disk cache file: "OXSL", version, width, height (u32 little-endian), then the RGBA
/// compressed (zlib, fastest level: 4 to 10 times smaller, read back in a few milliseconds).
pub(crate) fn cache_read(path: &Path) -> Option<Image> {
    let data = std::fs::read(path).ok()?;
    let u = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if data.get(..4)? != b"OXSL" || u(4)? != ART_VERSION {
        return None;
    }
    let (w, h) = (u(8)?, u(12)?);
    let rgba = unpack(data.get(16..)?, (w as usize) * (h as usize) * 4)?;
    Some(Image { w, h, rgba })
}

/// True when `path` is a cache file of this version (its header only is read).
pub fn cache_current(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 8];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && &head[..4] == b"OXSL" && u32::from_le_bytes([head[4], head[5], head[6], head[7]]) == ART_VERSION
}

/// `size` bytes compressed by `pack`; None when the data is damaged or of another size.
pub fn unpack(packed: &[u8], size: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::with_capacity(size);
    flate2::read::ZlibDecoder::new(packed).take(size as u64 + 1).read_to_end(&mut out).ok()?;
    (out.len() == size).then_some(out)
}

/// `bytes` compressed for a cache file (zlib, fastest level).
pub fn pack(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::with_capacity(bytes.len() / 4), flate2::Compression::fast());
    let _ = e.write_all(bytes);
    e.finish().unwrap_or_default()
}

pub(crate) fn cache_write(path: &Path, img: &Image) {
    let packed = pack(&img.rgba);
    let mut data = Vec::with_capacity(16 + packed.len());
    data.extend_from_slice(b"OXSL");
    for v in [ART_VERSION, img.w, img.h] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    data.extend_from_slice(&packed);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, &data).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Made once per run (memory), and from/to `cache_dir` when given. A failure is remembered too.
fn cached(slot: &'static OnceLock<Mutex<Option<Option<Arc<Image>>>>>, cache_file: Option<&Path>, make: impl FnOnce() -> Option<Image>) -> Option<Arc<Image>> {
    let cell = slot.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().ok()?;
    if let Some(hit) = guard.as_ref() {
        return hit.clone();
    }
    let img = cache_file.and_then(cache_read).or_else(|| {
        let made = make()?;
        if let Some(f) = cache_file {
            cache_write(f, &made);
        }
        Some(made)
    });
    let img = img.map(Arc::new);
    *guard = Some(img.clone());
    img
}

/// Jones for the Select level page (see `jones_from_textures`), cached.
pub fn jones(cache_dir: Option<&Path>, texture: impl FnMut(&str) -> Option<Image>) -> Option<Arc<Image>> {
    static SLOT: OnceLock<Mutex<Option<Option<Arc<Image>>>>> = OnceLock::new();
    let file = cache_dir.map(|d| d.join("select-level-jones.bin"));
    cached(&SLOT, file.as_deref(), || jones_from_textures(texture))
}

/// Unit of the enlargements: `SCALE_ONE` is the picture's own size. A tile of 128 texels (the
/// smallest of the menus' characters) then has a whole number of pixels at any enlargement.
pub const SCALE_ONE: u32 = 128;

/// Enlargement (in 1 / `SCALE_ONE`) of a picture shown `pixels_per_texel` times its size on the
/// screen: that size, at most 4 times; its own size when it is shown smaller (the game then
/// reduces a picture whose edge is already soft). None when it has not been drawn yet.
pub fn screen_scale(pixels_per_texel: f32) -> Option<u32> {
    (pixels_per_texel > 0.0).then(|| ((pixels_per_texel.max(1.0) * SCALE_ONE as f32).round() as u32).min(4 * SCALE_ONE))
}

/// Name part of a cache file for a size and an edge setting.
pub fn edge_tag(scale: u32, plain: bool) -> String {
    format!("q{scale}-{}", if plain { "soft" } else { "smooth" })
}

/// The pictures `jones_fine` made in this run, by size and variant.
static FINE: Mutex<Vec<((u32, bool), Option<Arc<Image>>)>> = Mutex::new(Vec::new());

/// Jones as `jones_fine` made her, if it is done (never waits nor makes her).
pub fn jones_fine_made(k: u32, plain: bool) -> Option<Option<Arc<Image>>> {
    FINE.lock().ok()?.iter().find(|(key, _)| *key == (k, plain)).map(|(_, img)| img.clone())
}

/// Jones with her edge anti-aliased (`smooth_edge`), at `k` / SCALE_ONE times the cut-out's size,
/// cached like `jones` (one picture per size and variant).
pub fn jones_fine(cache_dir: Option<&Path>, k: u32, plain: bool, texture: impl FnMut(&str) -> Option<Image>) -> Option<Arc<Image>> {
    if let Some((_, hit)) = FINE.lock().ok()?.iter().find(|(key, _)| *key == (k, plain)) {
        return hit.clone();
    }
    let file = cache_dir.map(|d| d.join(format!("select-level-jones-{}.bin", edge_tag(k, plain))));
    let img = file.as_deref().and_then(cache_read).map(Arc::new).or_else(|| {
        let made = smooth_edge_with(&*jones(cache_dir, texture)?, k, plain, &[], Some(JONES_EVEN));
        if let Some(f) = &file {
            cache_write(f, &made);
        }
        Some(Arc::new(made))
    });
    FINE.lock().ok()?.push(((k, plain), img.clone()));
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: [u8; 3] = KEY_EXPECTED;

    #[test]
    fn smooth_edge_turns_staircases_into_slopes_and_keeps_the_shape() {
        // A slope of one texel down every three across, as a staircase, over a solid base.
        let (w, h) = (40u32, 20u32);
        let mut img = Image::new(w, h);
        for x in 2..38 {
            let top = 4 + (x - 2) / 3;
            for y in top..18 {
                let i = at(&img, x, y);
                img.rgba[i..i + 4].copy_from_slice(&[200, 100, 50, 255]);
            }
        }
        let loops = trace_silhouette(&img);
        assert_eq!(loops.len(), 1);
        let smooth = smooth_loop(&loops[0]);
        // Never further than half a texel from the traced loop (a little more is the float's).
        let dist = |p: [f32; 2], a: [f32; 2], b: [f32; 2]| {
            let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
            let t = (((p[0] - a[0]) * vx + (p[1] - a[1]) * vy) / (vx * vx + vy * vy).max(1e-9)).clamp(0.0, 1.0);
            ((p[0] - a[0] - vx * t).powi(2) + (p[1] - a[1] - vy * t).powi(2)).sqrt()
        };
        let n = loops[0].len();
        for p in &smooth {
            let near = (0..n).map(|i| dist(*p, loops[0][i], loops[0][(i + 1) % n])).fold(f32::MAX, f32::min);
            assert!(near <= 1e-3, "{p:?} is off the loop by {near}");
        }
        for i in 0..n {
            let m = smooth.len();
            let near = (0..m).map(|k| dist(loops[0][i], smooth[k], smooth[(k + 1) % m])).fold(f32::MAX, f32::min);
            assert!(near <= 0.5 + 1e-3, "corner {:?} is {near} from the smoothed loop", loops[0][i]);
        }
        // The staircase's points lie on one line, through the middle of its first run (3.5, 4)
        // and of its first step (5, 4.5): y = 4 + (x - 3.5) / 3 between its ends.
        let on_slope: Vec<&[f32; 2]> = smooth.iter().filter(|p| p[1] < 15.0 && p[0] > 4.0 && p[0] < 36.0).collect();
        assert!(on_slope.len() >= 20);
        for p in &on_slope {
            assert!((p[1] - (4.0 + (p[0] - 3.5) / 3.0)).abs() < 1e-3, "{p:?}");
        }
        // The picture at three times its size: opaque inside with the drawing's colour, clear
        // outside, and along the slope a coverage that changes by little from a column to the next.
        let out = smooth_edge(&img, 3 * SCALE_ONE, false, &[]);
        assert_eq!((out.w, out.h), (120, 60));
        assert_eq!(get(&out, 60, 45), [200, 100, 50, 255]);
        assert_eq!(get(&out, 60, 3)[3], 0);
        let column = |x: u32| (0..40).map(|y| get(&out, x, y)[3] as i32).sum::<i32>();
        for x in 18..100 {
            assert!((column(x + 1) - column(x)).abs() <= 255 * 2 / 3, "column {x}");
        }
        // Same size: the solid texels are the drawing's own.
        // The simple variant: the same size, opaque inside, clear outside, partly clear on the edge.
        let plain = smooth_edge(&img, 3 * SCALE_ONE, true, &[]);
        assert_eq!((plain.w, plain.h), (120, 60));
        assert_eq!(get(&plain, 60, 45), [200, 100, 50, 255]);
        assert_eq!(get(&plain, 60, 3)[3], 0);
        assert!((0..40).any(|y| (1..255).contains(&get(&plain, 60, y)[3])));
        // A soft zone: the simple variant inside it, the slopes far from it.
        let zoned = smooth_edge(&img, 3 * SCALE_ONE, false, &[[2, 0, 12, 19]]);
        for y in 0..60 {
            assert_eq!(get(&zoned, 15, y), get(&plain, 15, y), "inside the zone, row {y}");
            assert_eq!(get(&zoned, 100, y), get(&out, 100, y), "far from the zone, row {y}");
        }
        // Sizes: the picture is made for the size it is shown at, never under its own.
        assert_eq!(screen_scale(0.0), None);
        assert_eq!(screen_scale(0.8), Some(SCALE_ONE));
        assert_eq!(screen_scale(1.0), Some(SCALE_ONE));
        assert_eq!(screen_scale(1.74), Some(223));
        assert_eq!(screen_scale(9.0), Some(4 * SCALE_ONE));
        let odd = smooth_edge(&img, 223, false, &[]);
        assert_eq!((odd.w, odd.h), (70, 35));
        let same = smooth_edge(&img, SCALE_ONE, false, &[]);
        assert_eq!((same.w, same.h), (w, h));
        assert_eq!(get(&same, 20, 12), [200, 100, 50, 255]);
    }

    fn solid(w: u32, h: u32, c: [u8; 3]) -> Image {
        Image { w, h, rgba: (0..w * h).flat_map(|_| [c[0], c[1], c[2], 255]).collect() }
    }

    fn put(img: &mut Image, x: u32, y: u32, c: [u8; 3]) {
        let i = at(img, x, y);
        img.rgba[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
    }

    fn get(img: &Image, x: u32, y: u32) -> [u8; 4] {
        let i = at(img, x, y);
        [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
    }

    #[test]
    fn quarters_are_laid_out_one_two_three_four() {
        let q: [Image; 4] = std::array::from_fn(|n| solid(4, 4, [n as u8 * 10, 0, 0]));
        let a = assemble_sized(&q, 4).unwrap();
        assert_eq!((a.w, a.h), (8, 8));
        assert_eq!(get(&a, 0, 0)[0], 0);
        assert_eq!(get(&a, 7, 0)[0], 10);
        assert_eq!(get(&a, 0, 7)[0], 20);
        assert_eq!(get(&a, 7, 7)[0], 30);
        assert_eq!(get(&a, 3, 3)[0], 0);
        assert_eq!(get(&a, 4, 4)[0], 30);
        // Wrong size: nothing.
        let mut bad = q.clone();
        bad[2] = solid(4, 5, [0; 3]);
        assert!(assemble_sized(&bad, 4).is_none());
        assert!(assemble(&q).is_none());
    }

    #[test]
    fn fill_stops_at_the_figure_and_skips_enclosed_pockets() {
        // 9 × 9 key with a ring of white (3..=5): its centre is a key pocket not reached.
        let mut img = solid(9, 9, K);
        for y in 3..=5 {
            for x in 3..=5 {
                put(&mut img, x, y, [255, 255, 255]);
            }
        }
        put(&mut img, 4, 4, K);
        // A near-key texel at the border is taken, a far one is not.
        put(&mut img, 0, 8, [K[0] - 10, K[1], K[2]]);
        put(&mut img, 8, 0, [0, 0, 0]);
        let bg = flood_background(&img, K, TOLERANCE, &[]);
        assert!(bg[0] && bg[8 * 9]);
        assert!(!bg[8]);
        assert!(!bg[4 * 9 + 4], "enclosed pocket left alone");
        assert!(!bg[3 * 9 + 3]);
        let seeded = flood_background(&img, K, TOLERANCE, &[[4, 4]]);
        assert!(seeded[4 * 9 + 4], "a seed reaches it");
    }

    #[test]
    fn edge_is_soft_and_unmixed() {
        // Left half key, right half green, with a 50 % mix column between.
        let (w, h) = (16, 8);
        let mut img = solid(w, h, K);
        let g = [40u8, 160, 40];
        for y in 0..h {
            for x in 9..w {
                put(&mut img, x, y, g);
            }
            put(&mut img, 8, y, [((K[0] as u32 + 40) / 2) as u8, ((K[1] as u32 + 160) / 2) as u8, ((K[2] as u32 + 40) / 2) as u8]);
        }
        let bg = flood_background(&img, K, TOLERANCE, &[]);
        let out = cut_out(&img, &bg, K);
        assert_eq!(get(&out, 2, 3)[3], 0);
        let mix = get(&out, 8, 3);
        assert!((mix[3] as i32 - 128).abs() <= 8, "half alpha: {mix:?}");
        assert!((mix[0] as i32 - 40).abs() <= 6 && (mix[1] as i32 - 160).abs() <= 6 && (mix[2] as i32 - 40).abs() <= 6, "colour unmixed: {mix:?}");
        assert_eq!(get(&out, 14, 3), [40, 160, 40, 255]);
    }

    #[test]
    fn despill_removes_magenta_cast_near_the_edge_only() {
        // Grey figure with a key-tinted column 4 texels in (DXT1 bleed): unmixed back to grey,
        // opaque; a far tinted texel is untouched.
        let (w, h) = (24, 6);
        let mut img = solid(w, h, K);
        for y in 0..h {
            for x in 5..w {
                put(&mut img, x, y, [120, 120, 120]);
            }
            put(&mut img, 8, y, [150, 90, 125]);
        }
        put(&mut img, 20, 2, [150, 90, 125]);
        let bg = flood_background(&img, K, TOLERANCE, &[]);
        let out = cut_out(&img, &bg, K);
        let near = get(&out, 8, 2);
        assert!(near[3] == 255 && near[0].min(near[2]) <= near[1], "near: {near:?}");
        assert_eq!(get(&out, 20, 2), [150, 90, 125, 255], "far texels untouched");
        // A crimson figure keeps its own cast at the edge.
        let mut img = solid(w, h, K);
        for y in 0..h {
            for x in 5..w {
                put(&mut img, x, y, [130, 28, 60]);
            }
        }
        let out = cut_out(&img, &flood_background(&img, K, TOLERANCE, &[]), K);
        assert_eq!(get(&out, 7, 2), [130, 28, 60, 255]);
    }

    #[test]
    fn dark_outline_stays_whole_next_to_a_light_fill() {
        // Key | outline column mixed 50 % with the key | black outline | white fill: the projection
        // on key -> white must not empty the outline.
        let (w, h) = (20, 6);
        let mut img = solid(w, h, K);
        for y in 0..h {
            put(&mut img, 6, y, [K[0] / 2, K[1] / 2, K[2] / 2]);
            put(&mut img, 7, y, [0, 0, 0]);
            for x in 8..w {
                put(&mut img, x, y, [250, 250, 250]);
            }
        }
        let bg = flood_background(&img, K, TOLERANCE, &[]);
        let out = cut_out(&img, &bg, K);
        assert_eq!(get(&out, 7, 2), [0, 0, 0, 255], "outline solid");
        let mix = get(&out, 6, 2);
        assert!(mix[3] == 255 && mix[0].max(mix[1]).max(mix[2]) <= 10, "mixed outline texel black and solid: {mix:?}");
        assert_eq!(get(&out, 3, 2)[3], 0);
        assert_eq!(get(&out, 15, 2), [250, 250, 250, 255]);
        // A light texel half mixed with the key, white beyond: half alpha, white colour.
        let mut img = solid(w, h, K);
        for y in 0..h {
            put(&mut img, 6, y, [((K[0] as u32 + 250) / 2) as u8, ((K[1] as u32 + 250) / 2) as u8, ((K[2] as u32 + 250) / 2) as u8]);
            for x in 7..w {
                put(&mut img, x, y, [250, 250, 250]);
            }
        }
        let out = cut_out(&img, &flood_background(&img, K, TOLERANCE, &[]), K);
        let m = get(&out, 6, 2);
        assert!((m[3] as i32 - 128).abs() <= 8 && m[0].min(m[1]).min(m[2]) >= 240, "light edge: {m:?}");
    }

    #[test]
    fn key_like_texels_deep_inside_are_untouched() {
        // A pink spot (near the key, but not connected to the background) inside a grey block.
        let mut img = solid(24, 24, K);
        for y in 4..20 {
            for x in 4..20 {
                put(&mut img, x, y, [100, 100, 100]);
            }
        }
        put(&mut img, 12, 12, [K[0] - 5, K[1] + 5, K[2]]);
        let bg = flood_background(&img, K, TOLERANCE, &[]);
        let out = cut_out(&img, &bg, K);
        assert_eq!(get(&out, 12, 12), [K[0] - 5, K[1] + 5, K[2], 255]);
        assert_eq!(get(&out, 10, 10), [100, 100, 100, 255]);
    }

    #[test]
    fn specks_cut_off_are_dropped() {
        let mut img = solid(30, 30, K);
        for y in 10..25 {
            for x in 10..25 {
                put(&mut img, x, y, [60, 120, 60]);
            }
        }
        put(&mut img, 3, 3, [60, 120, 60]);
        put(&mut img, 4, 3, [60, 120, 60]);
        let bg = flood_background(&img, K, TOLERANCE, &[]);
        let out = cut_out(&img, &bg, K);
        assert_eq!(get(&out, 3, 3)[3], 0, "speck dropped");
        assert_eq!(get(&out, 17, 17), [60, 120, 60, 255]);
        let mut kept = out.clone();
        drop_specks(&mut kept, 1);
        assert_eq!(kept, out);
    }

    #[test]
    fn the_edge_is_cleaned_without_touching_the_drawing() {
        // A red shape inside a black outline two texels thick, on a transparent picture.
        let set = |img: &mut Image, x: u32, y: u32, c: [u8; 4]| {
            let i = at(img, x, y);
            img.rgba[i..i + 4].copy_from_slice(&c);
        };
        const INK: [u8; 4] = [8, 0, 0, 255];
        let mut img = Image::new(40, 40);
        for y in 5..35 {
            for x in 5..35 {
                let edge = !(7..33).contains(&x) || !(7..33).contains(&y);
                set(&mut img, x, y, if edge { INK } else { [150, 30, 30, 255] });
            }
        }
        // A gap in the outline: the colour behind it is not a stray.
        set(&mut img, 5, 20, [150, 30, 30, 255]);
        set(&mut img, 6, 20, [150, 30, 30, 255]);
        let clean = img.clone();
        let mut same = clean.clone();
        clean_edge(&mut same);
        assert_eq!(same, clean, "a clean picture is left as it is");
        // Strays above the outline: solid ones stacked, a faint one beside; a faint dark texel
        // away from anything solid; a bump of the outline two texels long.
        set(&mut img, 8, 4, [100, 28, 28, 255]);
        set(&mut img, 8, 3, [88, 63, 63, 255]);
        set(&mut img, 9, 4, [164, 35, 10, 97]);
        set(&mut img, 20, 2, [10, 0, 0, 60]);
        set(&mut img, 35, 12, INK);
        set(&mut img, 35, 13, INK);
        // A faint dark texel on the outline is its soft edge: kept. A longer step of the
        // outline is its shape: kept.
        set(&mut img, 14, 4, [16, 7, 0, 100]);
        for x in 20..26 {
            set(&mut img, x, 35, INK);
        }
        // A stray in the corner of a step of the outline.
        for x in 28..33 {
            set(&mut img, x, 4, INK);
        }
        set(&mut img, 27, 4, [100, 28, 28, 255]);
        let mut out = img.clone();
        clean_edge(&mut out);
        // The strays on the outline take its colour and keep their alpha; the others go.
        // (alone on a straight edge, the repainted one is a bump and goes too).
        assert_eq!(get(&out, 27, 4), INK);
        assert_eq!(get(&out, 9, 4), [8, 0, 0, 97]);
        for (x, y) in [(8, 4), (8, 3), (20, 2), (35, 12), (35, 13)] {
            assert_eq!(get(&out, x, y)[3], 0, "dropped {x},{y}");
        }
        assert_eq!(get(&out, 14, 4), [16, 7, 0, 100]);
        for x in 20..26 {
            assert_eq!(get(&out, x, 35)[3], 255, "step kept {x}");
        }
        for y in 5..35 {
            for x in 5..35 {
                assert_eq!(get(&out, x, y), get(&clean, x, y), "{x},{y}");
            }
        }
        // A bump on an outline one texel thick stays: the line would break.
        let mut thin = Image::new(40, 40);
        for y in 5..35 {
            for x in 5..35 {
                let edge = x == 5 || x == 34 || y == 5 || y == 34;
                set(&mut thin, x, y, if edge { INK } else { [150, 30, 30, 255] });
            }
        }
        set(&mut thin, 35, 12, INK);
        let before = thin.clone();
        clean_edge(&mut thin);
        assert_eq!(thin, before);
        // A shape without outline keeps its rim.
        let mut plain = Image::new(40, 40);
        for y in 5..35 {
            for x in 5..35 {
                set(&mut plain, x, y, [150, 30, 30, 255]);
            }
        }
        let before = plain.clone();
        clean_edge(&mut plain);
        assert_eq!(plain, before);
    }

    #[test]
    fn jones_stands_on_the_background_bottom_edge() {
        let mut img = Image::new(100, 80);
        for y in 30..80 {
            for x in 40..60 {
                let i = at(&img, x, y);
                img.rgba[i + 3] = 255;
            }
        }
        let f = figure_bounds(&img).unwrap();
        assert_eq!(f, [40, 30, 59, 79]);
        assert!(figure_bounds(&Image::new(4, 4)).is_none());
        for bgr in [[282.0, 219.0, 439.0, 495.0], [10.0, 20.0, 200.0, 150.0]] {
            let [x, y, w, h] = jones_rect(bgr, (img.w, img.h), f);
            let s = w / img.w as f32;
            assert!((h / img.h as f32 - s).abs() < 1e-4, "aspect kept");
            assert!((y + (f[3] + 1) as f32 * s - (bgr[1] + bgr[3])).abs() < 1e-3, "feet on the bottom edge");
            assert!(((f[3] + 1 - f[1]) as f32 * s - FIGURE_HEIGHT * bgr[3]).abs() < 1e-3, "figure height");
            assert!((x + f[0] as f32 * s - (bgr[0] + bgr[2] - FIGURE_OVERLAP * bgr[3])).abs() < 1e-3);
        }
    }

    #[test]
    fn mirror_and_cut() {
        let mut img = solid(3, 4, [0; 3]);
        put(&mut img, 0, 0, [9, 9, 9]);
        put(&mut img, 2, 3, [7, 7, 7]);
        let m = mirror_h(&img);
        assert_eq!(get(&m, 2, 0), [9, 9, 9, 255]);
        assert_eq!(get(&m, 0, 3), [7, 7, 7, 255]);
        assert_eq!(mirror_h(&m), img);
        let c = cut_bottom(&img, 1);
        assert_eq!((c.w, c.h), (3, 3));
        assert_eq!(get(&c, 0, 0), [9, 9, 9, 255]);
        assert_eq!(cut_bottom(&img, 10).h, 0);
    }

    #[test]
    fn paint_rects_clips() {
        let mut img = solid(4, 4, [0; 3]);
        paint_rects(&mut img, &[[2, 2, 10, 10]], [5, 6, 7]);
        assert_eq!(get(&img, 3, 3), [5, 6, 7, 255]);
        assert_eq!(get(&img, 1, 1), [0, 0, 0, 255]);
    }

    #[test]
    fn wrong_inputs_give_no_character_or_background() {
        let q: [Image; 4] = std::array::from_fn(|_| solid(128, 128, K));
        assert!(jones_from_quarters(&q).is_none(), "size");
        let q: [Image; 4] = std::array::from_fn(|_| solid(QUARTER, QUARTER, [0, 255, 0]));
        assert!(jones_from_quarters(&q).is_none(), "background colour");
        let q: [Image; 4] = std::array::from_fn(|_| solid(QUARTER, QUARTER, K));
        assert!(jones_from_quarters(&q).is_none(), "nothing left");
        assert!(background_from_frame(&solid(320, 240, [0; 3])).is_none());
        let b = background_from_frame(&solid(640, 480, [1, 2, 3])).unwrap();
        assert_eq!((b.w, b.h), (BACKGROUND_CROP[2], BACKGROUND_CROP[3]));
    }

    #[test]
    fn synthetic_jones_is_cut_mirrored_and_trimmed() {
        // A figure (grey block) on the key, quarters made from one 512 picture.
        let s = QUARTER * 2;
        let mut whole = solid(s, s, K);
        for y in 100..s {
            for x in 150..330 {
                put(&mut whole, x, y, [90, 110, 70]);
            }
        }
        // "Text" inside the erased rectangle.
        put(&mut whole, 40, 390, [0, 0, 0]);
        let q: [Image; 4] = std::array::from_fn(|n| whole.region((n as u32 % 2) * QUARTER, (n as u32 / 2) * QUARTER, QUARTER, QUARTER));
        let j = jones_from_quarters(&q).unwrap();
        assert_eq!((j.w, j.h), (s, s - CUT_BOTTOM));
        assert_eq!(get(&j, s - 1 - 40, 390)[3], 0, "text erased");
        assert_eq!(get(&j, s - 1 - 200, 300), [90, 110, 70, 255], "figure mirrored");
        assert_eq!(get(&j, 100, 300)[3], 0, "key around the figure");
    }

    #[test]
    fn disk_cache_round_trip() {
        let dir = std::env::temp_dir().join(format!("oxiii-sl-cache-{}", std::process::id()));
        let file = dir.join("x.bin");
        let img = solid(3, 2, [1, 2, 3]);
        cache_write(&file, &img);
        assert_eq!(cache_read(&file), Some(img));
        std::fs::write(&file, b"OXSL\x00\x00\x00\x00").unwrap();
        assert_eq!(cache_read(&file), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Previews from the player's game (OXIII_GAME; skipped when not set) in the system's temporary
    /// folder (openxiii-select-level), with the time each picture takes.
    #[test]
    fn previews_from_the_game() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| openxiii_package::system_dir(g).exists()) else {
            eprintln!("OXIII_GAME not set: nothing to make");
            return;
        };
        let out_dir = std::env::temp_dir().join("openxiii-select-level");
        eprintln!("previews in {}", out_dir.display());
        std::fs::create_dir_all(&out_dir).unwrap();
        let t0 = std::time::Instant::now();
        let mut library = openxiii_package::library::Library::new(&game).unwrap();
        let t_lib = t0.elapsed();
        let t1 = std::time::Instant::now();
        let mut read = |path: &str| {
            let (loaded, index) = library.resolve(path)?;
            let t = openxiii_assets::texture::Texture::read(&loaded.package, &loaded.data, index).ok()?;
            let (w, h, rgba) = t.d3d_mips().into_iter().next()?;
            let opaque = rgba.chunks_exact(4).filter(|p| p[3] == 255).count();
            eprintln!("{path}: {:?}, {} mips, {w}x{h}, opaque texels {opaque}/{}", t.format, t.mips.len(), w * h);
            Some(Image { w, h, rgba })
        };
        let quarters = [read(JONES_TEXTURES[0]).unwrap(), read(JONES_TEXTURES[1]).unwrap(), read(JONES_TEXTURES[2]).unwrap(), read(JONES_TEXTURES[3]).unwrap()];
        let t_read = t1.elapsed();
        let t2 = std::time::Instant::now();
        let jones = jones_from_quarters(&quarters).expect("cut-out");
        let t_cut = t2.elapsed();
        let video_dir = game.join("Video");
        let t3 = std::time::Instant::now();
        let frame = crate::still_frames::wait(&background_video(&video_dir), BACKGROUND_FRAME, std::time::Duration::from_secs(60)).expect("Cine14 frame 2000 (ffmpeg)");
        let bg = background_from_frame(&Image { w: frame.w, h: frame.h, rgba: frame.rgba.clone() }).unwrap();
        let t_bg = t3.elapsed();
        eprintln!("library {t_lib:?}; Jones: textures {t_read:?}, cut-out {t_cut:?}; background (ffmpeg) {t_bg:?}");
        // Disk cache: a second making reads the files.
        let cache = out_dir.join("cache");
        let _ = std::fs::remove_dir_all(&cache);
        // A slot of this test's own: `jones` keeps one Jones for the whole process, and when
        // another test of the same run (select_level_page) made her first, she came from that
        // memory without the cache file being written here.
        static SLOT: OnceLock<Mutex<Option<Option<Arc<Image>>>>> = OnceLock::new();
        let j1 = super::cached(&SLOT, Some(&cache.join("select-level-jones.bin")), || super::jones_from_textures(|p| read(p))).unwrap();
        assert_eq!(background(&video_dir).unwrap(), bg);
        let t4 = std::time::Instant::now();
        let jc = cache_read(&cache.join("select-level-jones.bin")).unwrap();
        eprintln!("Jones from the disk cache: {:?}", t4.elapsed());
        assert_eq!(jc, *j1);
        assert_eq!(*j1, jones);
        let _ = std::fs::remove_dir_all(&cache);

        let save = |name: &str, img: &Image| openxiii_assets::texture::write_png(&out_dir.join(name), img.w, img.h, &img.rgba).unwrap();
        // Over a checkerboard.
        let mut checker = Image::new(jones.w, jones.h);
        for y in 0..jones.h {
            for x in 0..jones.w {
                let c = if (x / 8 + y / 8) % 2 == 0 { 204 } else { 128 };
                let i = at(&checker, x, y);
                checker.rgba[i..i + 4].copy_from_slice(&[c, c, c, 255]);
            }
        }
        save("jones-raw.png", &mirror_h(&assemble(&quarters).unwrap()));
        save("jones-cutout.png", &jones);
        for k in [2, 4] {
            let t = std::time::Instant::now();
            let fine = smooth_edge(&jones, k * SCALE_ONE, false, &[]);
            eprintln!("edge redrawn x{k}: {:?}, {} loops", t.elapsed(), trace_silhouette(&jones).len());
            save(&format!("jones-fine-x{k}.png"), &fine);
        }
        save("jones-cutout-checker.png", &over(&checker, &jones, 0, 0));
        save("background-cine14-2000.png", &bg);
        // Mock-up composite (1280 × 960): white page, the background where the mock-up has it,
        // Jones placed by `jones_rect` (her lowest row on the background's bottom row).
        let page = solid(1280, 960, [252, 252, 252]);
        let rect = [282.0, 219.0, 439.0, 495.0];
        let page = over(&page, &resample(&bg, 439, 495), 282, 219);
        let figure = figure_bounds(&jones).unwrap();
        assert_eq!(figure[3], jones.h - 1, "figure reaches the picture's bottom row");
        let [jx, jy, jw, jh] = jones_rect(rect, (jones.w, jones.h), figure);
        let js = resample(&jones, jw.round() as u32, jh.round() as u32);
        let bottom = (rect[1] + rect[3]) as i32;
        let page = over(&page, &js, jx.round() as i32, bottom - js.h as i32);
        let f2 = figure_bounds(&js).unwrap();
        eprintln!("Jones at x {jx:.1} y {jy:.1} ({jw:.1} × {jh:.1}); figure rows {}..{} on the page, background rows {}..{}", jy.round() as i32 + f2[1] as i32, bottom - js.h as i32 + f2[3] as i32, rect[1], bottom - 1);
        save("mock-up-composite.png", &page);
    }

    /// Premultiplied bilinear resampling (as the page's texture filter draws it).
    fn resample(img: &Image, w: u32, h: u32) -> Image {
        let mut out = Image::new(w, h);
        let px = |x: i32, y: i32| {
            let i = at(img, x.clamp(0, img.w as i32 - 1) as u32, y.clamp(0, img.h as i32 - 1) as u32);
            let a = img.rgba[i + 3] as f32 / 255.0;
            [img.rgba[i] as f32 * a, img.rgba[i + 1] as f32 * a, img.rgba[i + 2] as f32 * a, a]
        };
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = ((x as f32 + 0.5) * img.w as f32 / w as f32 - 0.5, (y as f32 + 0.5) * img.h as f32 / h as f32 - 0.5);
                let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
                let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                let mut c = [0.0f32; 4];
                for (dx, dy, wt) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
                    let p = px(x0 + dx, y0 + dy);
                    (0..4).for_each(|k| c[k] += p[k] * wt);
                }
                let d = at(&out, x, y);
                for k in 0..3 {
                    out.rgba[d + k] = if c[3] > 0.0 { (c[k] / c[3]).round().clamp(0.0, 255.0) as u8 } else { 0 };
                }
                out.rgba[d + 3] = (c[3] * 255.0).round() as u8;
            }
        }
        out
    }

    /// `top` drawn over `base` at (x, y) with its alpha.
    fn over(base: &Image, top: &Image, x: i32, y: i32) -> Image {
        let mut out = base.clone();
        for ty in 0..top.h as i32 {
            for tx in 0..top.w as i32 {
                let (bx, by) = (x + tx, y + ty);
                if bx < 0 || by < 0 || bx >= base.w as i32 || by >= base.h as i32 {
                    continue;
                }
                let (s, d) = (at(top, tx as u32, ty as u32), at(&out, bx as u32, by as u32));
                let a = top.rgba[s + 3] as f32 / 255.0;
                for k in 0..3 {
                    out.rgba[d + k] = (top.rgba[s + k] as f32 * a + out.rgba[d + k] as f32 * (1.0 - a)).round() as u8;
                }
                out.rgba[d + 3] = 255;
            }
        }
        out
    }
}
