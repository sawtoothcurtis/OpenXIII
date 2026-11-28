//! Mods build only (feature `mods`): first-person arms made at load time from a game mesh
//! (`[arms.<Name>]` of a mod, docs/modding/README.md, "Other arms in first person").
//!
//! Nothing is shipped with the mod: the recipe reads the first-person mesh (arms and weapon) and
//! its textures in the player's game, then
//! - makes the fingers, hands, wrists and forearms thinner, in the reference pose: each point
//!   moves towards the axis of the bones that hold it, weighted by their skinning weights, so
//!   the game's own animations still play on it. The weapon's bones are left untouched;
//! - recolours the arms' texture to another skin tone (given, or measured on the hands and
//!   forearms of a character's model), keeping its drawing and shading, and tints the nails.
//!
//! The result is a [`ModSkeletal`] that keeps the game's animation, attachment names and outline
//! bytes, and a new texture for the arms.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::collections::HashMap;

use openxiii_package::library::Library;
use openxiii_package::ObjectRef;

use crate::mod_skeletal::ModSkeletal;
use crate::skeletal::SkeletalMesh;

/// The skin tone of the new arms.
#[derive(Debug, Clone, PartialEq)]
pub enum Skin {
    /// The game's colours.
    Unchanged,
    /// This average colour (RGB).
    Color([u8; 3]),
    /// The colour measured on the hands and forearms of this character model ("XIIIPersos.JonesM").
    From(String),
}

/// Thickness factors (1 = unchanged): fingers, hands (palm and back), wrists, forearms (at the
/// elbow; the forearm goes from this one to the wrist's along its length).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub fingers: f32,
    pub hand: f32,
    pub wrist: f32,
    pub forearm: f32,
}

/// How to make the arms.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    /// The game's first-person meshes to start from: a mesh ("XIIIArmes.FpsBerrettaM") or every
    /// mesh of a package that has first-person arm bones ("XIIIArmes.*").
    pub from: Vec<String>,
    /// Meshes left out (when a package is given).
    pub skip: Vec<String>,
    pub shape: Shape,
    /// Another shape for some meshes.
    pub per_mesh: Vec<(String, Shape)>,
    /// Another place for the sleeve's cuff on some meshes (a sleeve riding up the arm when it
    /// holds a long weapon far forward).
    pub per_mesh_cuff: Vec<(String, f32)>,
    pub skin: Skin,
    /// Colour of the nails (None: the skin's colour as the game draws it).
    pub nails: Option<[u8; 3]>,
    /// A sleeve over the forearm (None: bare arms, as the game's).
    pub sleeve: Option<Sleeve>,
    /// Gloves on the hands (None: bare hands).
    pub gloves: Option<Gloves>,
    /// An arm not drawn at all (a one-armed character): its faces are left out of every mesh,
    /// whatever it holds stays.
    pub hide_arm: Option<Side>,
}

/// A side of the first-person arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Gloves: the hands' texels past `from` along the forearm (0 elbow, 1 wrist) take the gloves'
/// colour instead of the skin's, with the hands' own drawing (fingers, knuckles).
#[derive(Debug, Clone, PartialEq)]
pub struct Gloves {
    /// Given, or measured on a character model's hands.
    pub colour: Skin,
    pub from: f32,
    /// Where they stop (fingerless gloves: before the fingers; None: on the whole hand).
    pub to: Option<f32>,
}

/// A sleeve drawn on the forearm, by the texture: every texel of the arms texture is placed
/// along the forearm from the meshes (0 at the elbow, 1 at the wrist); those before `cuff` take
/// the fabric's colour with the game's shading, the last `band` of them the band's colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Sleeve {
    /// The fabric's colour, given or measured on a character model's forearms.
    pub fabric: Skin,
    /// Where the sleeve ends along the forearm (0 elbow … 1 wrist).
    pub cuff: f32,
    /// Colour and width (along the forearm) of a band at the sleeve's edge (a cuff, a stripe).
    pub band: Option<([u8; 3], f32)>,
    /// Where the band starts along the forearm (None: it ends at the cuff).
    pub band_at: Option<f32>,
    /// A couple of thin crease lines drawn as the game's ink.
    pub creases: bool,
    /// Thin ink lines all around the sleeve, at these places along the forearm (seams).
    pub rings: Vec<f32>,
    /// The rings drawn in this colour and width (along the forearm) instead (braid stripes).
    pub ring_colour: Option<([u8; 3], f32)>,
    /// A strap tab on the sleeve, outlined in ink: from, to (along the forearm), its middle's
    /// angle around the forearm and half its width (radians).
    pub tab: Option<[f32; 4]>,
    /// A ribbed knit cuff: stripes along the forearm in this colour, from `[0]` along the
    /// forearm to the cuff, `[1]` of them around the arm, each `[2]` of its period wide.
    pub ribs: Option<([u8; 3], [f32; 3])>,
    /// A check pattern (a plaid): lines of this colour around the sleeve every `[0]` along the
    /// forearm and `[1]` lines along it, each `[2]` of its period wide.
    pub checks: Option<([u8; 3], [f32; 3])>,
    /// Buttons of this colour, outlined in ink, on the sleeve near the cuff: the first at `[0]`
    /// along the forearm, `[1]` of them `[3]` apart towards the elbow, at angle `[2]` around it.
    pub buttons: Option<([u8; 3], [f32; 4])>,
    /// A sleeve in relief over the painted one (None: painted only).
    pub model: Option<SleeveModel>,
}

/// A sleeve in relief: a shell built around each forearm, from its points, skinned on the
/// forearm bone, ending at the cuff with a thick lip, and drawn with the sleeve of a character
/// model's texture (its texture coordinates fitted on that model's forearm).
#[derive(Debug, Clone, PartialEq)]
pub struct SleeveModel {
    /// The character model whose sleeve (texture and its place on it) is used.
    pub from: String,
    /// The shell's radius as a factor of the arm's (1.12: 12 % wider).
    pub thickness: f32,
    /// Added room, in the mesh's units.
    pub room: f32,
    /// The lip at the cuff, as a factor of the shell's radius.
    pub lip: f32,
    /// Turns the texture around the arm (in turns, 0 to 1).
    pub seam: f32,
    /// Drawn with the painted sleeve (plain, in the fabric's colour) instead of the model's sleeve.
    pub painted: bool,
    /// Folds modelled in the shell: how many, their depth (fraction of the radius), width (along
    /// the forearm, 0 elbow … 1 wrist) and slant (how much they lean around the arm).
    pub folds: usize,
    pub fold_depth: f32,
    pub fold_width: f32,
    pub fold_slant: f32,
    /// A sleeve that narrows towards the cuff (a knit cuff hugging the wrist): from where along
    /// the forearm, and the radius at the cuff as a factor of the straight sleeve's.
    pub taper: Option<(f32, f32)>,
}

/// Arms made by [`build`]: each game mesh used and the mesh made from it, and the new textures
/// (name, width, height, RGBA).
pub struct Built {
    pub meshes: Vec<(String, ModSkeletal)>,
    pub textures: Vec<(String, u32, u32, Vec<u8>)>,
    /// What was measured or changed, for the log.
    pub notes: Vec<String>,
}

/// Bone kinds of the first-person arms (FPS_* right hand, FPSg_* left hand).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Finger,
    Hand,
    Forearm,
    Other,
}

fn kind(name: &str) -> Kind {
    let n = name.to_ascii_lowercase();
    if !n.starts_with("fps") {
        return Kind::Other;
    }
    if ["pouce", "index", "majeur", "anul", "oricu"].iter().any(|f| n.contains(f)) {
        Kind::Finger
    } else if n.contains("main") {
        Kind::Hand
    } else if n.contains("bras") {
        Kind::Forearm
    } else {
        Kind::Other
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn qualified(loaded_name: &str, package: &openxiii_package::Package, r: i32) -> String {
    match ObjectRef::from(r) {
        ObjectRef::None => String::new(),
        ObjectRef::Export(_) => format!("{loaded_name}.{}", package.object_path(r.into())),
        _ => package.object_path(r.into()),
    }
}

/// A game skeletal mesh, its bone names and qualified texture paths, its default animation, its
/// attachment names and its UPrimitive outline bytes.
type GameMesh = (SkeletalMesh, Vec<String>, Vec<String>, String, Vec<(String, String)>, [u8; 8]);

fn read_mesh(library: &mut Library, path: &str) -> Result<GameMesh, String> {
    let (loaded, index) = library.resolve(path).ok_or_else(|| format!("{path} not found in the game"))?;
    if loaded.package.export_class(index) != "SkeletalMesh" {
        return Err(format!("{path} is not an animated mesh"));
    }
    let (p, d) = (&loaded.package, &loaded.data);
    let mesh = SkeletalMesh::read(p, d, index).map_err(|e| format!("{path}: {e}"))?;
    let bones = mesh.bones.iter().map(|b| p.name(b.name).to_string()).collect();
    let textures = mesh.textures.iter().map(|&t| qualified(&loaded.name, p, t)).collect();
    let animation = qualified(&loaded.name, p, mesh.animation);
    let aliases = crate::skeletal_collision::read_attach_tags(p, d, index)
        .map(|t| t.aliases.iter().zip(&t.names).map(|(&a, &n)| (p.name(a).to_string(), p.name(n).to_string())).collect())
        .unwrap_or_default();
    let cartoon = crate::static_mesh::primitive_cartoon(p, d, index).map_err(|e| format!("{path}: {e}"))?;
    Ok((mesh, bones, textures, animation, aliases, cartoon))
}

/// A character model to measure: "Package.Mesh", or "Package.Mesh@Package.Texture" for the model
/// with another of its skins (the texture its maps give it instead of its own).
fn read_character(library: &mut Library, model: &str) -> Result<(SkeletalMesh, Vec<String>, Vec<String>), String> {
    let (path, skin) = match model.split_once('@') {
        Some((m, t)) => (m.trim(), Some(t.trim())),
        None => (model, None),
    };
    let (mesh, bones, mut textures, ..) = read_mesh(library, path)?;
    if let Some(skin) = skin {
        if library.resolve(skin).is_none() {
            return Err(format!("{skin} not found in the game"));
        }
        // Skins[0]: the texture of the model's first material.
        if let Some(t) = textures.first_mut() {
            *t = skin.to_string();
        }
    }
    Ok((mesh, bones, textures))
}

fn read_texture(library: &mut Library, path: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let (loaded, index) = library.resolve(path).ok_or_else(|| format!("texture {path} not found in the game"))?;
    if loaded.package.export_class(index) != "Texture" {
        return Err(format!("{path} is not a texture"));
    }
    crate::texture::Texture::read(&loaded.package, &loaded.data, index).and_then(|t| t.decode(0)).map_err(|e| format!("texture {path}: {e:?}"))
}

/// Texture index (into the mesh's textures) of each face.
fn face_textures(mesh: &SkeletalMesh) -> Vec<usize> {
    mesh.faces.iter().map(|f| mesh.materials.get(f.material as usize).map_or(f.material as i32, |m| m.1).max(0) as usize).collect()
}

/// The arms' texture of a first-person mesh: the one under most faces held by finger bones.
fn arms_texture(mesh: &SkeletalMesh, bones: &[String], textures: usize) -> Option<usize> {
    let tex_of = face_textures(mesh);
    let mut count = vec![0usize; textures];
    for (f, face) in mesh.faces.iter().enumerate() {
        let p = mesh.wedges[face.wedges[0] as usize].point as usize;
        if main_bone(&mesh.influences[p]).is_some_and(|b| bones.get(b).is_some_and(|n| kind(n) == Kind::Finger)) {
            if let Some(c) = count.get_mut(tex_of[f]) {
                *c += 1;
            }
        }
    }
    count.iter().enumerate().max_by_key(|&(_, c)| *c).filter(|&(_, &c)| c > 0).map(|(i, _)| i)
}

/// The strongest bone of a point.
fn main_bone(inf: &[(u16, f32)]) -> Option<usize> {
    inf.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|&(b, _)| b as usize)
}

/// Median colour of the texels under the faces of `mesh` held by bones named like a hand, a
/// finger or a forearm (a character model of the game).
fn measure_skin(library: &mut Library, model: &str) -> Result<[f32; 3], String> {
    measure(library, model, &["forearm", "hand", "finger"])
}

/// Median colour of the texels under the faces of a character model held by bones whose name
/// contains one of `parts` (lower case).
fn measure(library: &mut Library, model: &str, parts: &[&str]) -> Result<[f32; 3], String> {
    Ok(median(&measure_samples(library, model, parts)?))
}

/// As [`measure`], on the lighter half of the texels (their painted shadows left out).
fn measure_lit(library: &mut Library, model: &str, parts: &[&str]) -> Result<[f32; 3], String> {
    let mut s = measure_samples(library, model, parts)?;
    s.sort_by_key(|c| c[0] as u32 + c[1] as u32 + c[2] as u32);
    Ok(median(&s[s.len() / 2..]))
}

fn measure_samples(library: &mut Library, model: &str, parts: &[&str]) -> Result<Vec<[u8; 3]>, String> {
    let (mesh, bones, textures) = read_character(library, model)?;
    let wanted: Vec<bool> = bones
        .iter()
        .map(|n| {
            let n = n.to_ascii_lowercase();
            parts.iter().any(|p| n.contains(p))
        })
        .collect();
    if !wanted.iter().any(|&w| w) {
        return Err(format!("{model} has no {} bone to measure a colour on", parts.join(" or ")));
    }
    let tex_of = face_textures(&mesh);
    let mut images: HashMap<usize, (u32, u32, Vec<u8>)> = HashMap::new();
    let mut samples: Vec<[u8; 3]> = Vec::new();
    for (f, face) in mesh.faces.iter().enumerate() {
        let pts = face.wedges.map(|w| mesh.wedges[w as usize].point as usize);
        if !pts.iter().all(|&p| main_bone(&mesh.influences[p]).is_some_and(|b| wanted.get(b).copied().unwrap_or(false))) {
            continue;
        }
        let t = tex_of[f];
        if !images.contains_key(&t) {
            let Some(path) = textures.get(t) else { continue };
            images.insert(t, read_texture(library, path)?);
        }
        let (w, h, rgba) = &images[&t];
        let uvs = face.wedges.map(|k| mesh.wedges[k as usize].uv);
        // The three corners and the middle of the face.
        let mid = [(uvs[0][0] + uvs[1][0] + uvs[2][0]) / 3.0, (uvs[0][1] + uvs[1][1] + uvs[2][1]) / 3.0];
        for uv in [uvs[0], uvs[1], uvs[2], mid] {
            let x = ((uv[0].rem_euclid(1.0) * *w as f32) as u32).min(w - 1);
            let y = ((uv[1].rem_euclid(1.0) * *h as f32) as u32).min(h - 1);
            let i = ((y * w + x) * 4) as usize;
            samples.push([rgba[i], rgba[i + 1], rgba[i + 2]]);
        }
    }
    if samples.is_empty() {
        return Err(format!("{model}: no face on its {}", parts.join(" or ")));
    }
    Ok(samples)
}

fn median(samples: &[[u8; 3]]) -> [f32; 3] {
    // The value at the middle of the sorted list, by counting.
    [0, 1, 2].map(|c| {
        let mut count = [0usize; 256];
        for s in samples {
            count[s[c] as usize] += 1;
        }
        let mut left = samples.len() / 2;
        for (v, &n) in count.iter().enumerate() {
            if left < n {
                return v as f32;
            }
            left -= n;
        }
        unreachable!("median of no sample")
    })
}

/// Moves the points of the arms towards the axes of their bones (reference pose).
fn thin(mesh: &mut SkeletalMesh, bones: &[String], r: &Shape) {
    let bases = mesh.ref_space_bases();
    let pos: Vec<[f32; 3]> = bases.iter().map(|b| b.origin).collect();
    let n = mesh.bones.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 1..n {
        let p = mesh.bones[i].parent as usize;
        if p < n && p != i {
            children[p].push(i);
        }
    }
    // Axis of each bone: from its position to its first child of the same kind (the next
    // joint), otherwise the direction from its parent; with the bone's length.
    let kinds: Vec<Kind> = bones.iter().map(|b| kind(b)).collect();
    let axes: Vec<Option<([f32; 3], f32)>> = (0..n)
        .map(|b| {
            if kinds[b] == Kind::Other {
                return None;
            }
            let towards = children[b]
                .iter()
                .find(|&&c| kinds[c] == kinds[b] || (kinds[b] == Kind::Forearm && kinds[c] == Kind::Hand) || (kinds[b] == Kind::Hand && kinds[c] == Kind::Hand))
                .map(|&c| sub(pos[c], pos[b]));
            let d = towards.or_else(|| {
                let p = mesh.bones[b].parent as usize;
                (b > 0 && p < n).then(|| sub(pos[b], pos[p]))
            })?;
            let len = dot(d, d).sqrt();
            (len > 1e-4).then(|| (d.map(|c| c / len), if towards.is_some() { len } else { 0.0 }))
        })
        .collect();
    let factor = |b: usize, along: f32| -> f32 {
        match kinds[b] {
            Kind::Finger => r.fingers,
            Kind::Hand => r.hand,
            Kind::Forearm => {
                // Elbow → wrist along the bone.
                match axes[b] {
                    Some((_, len)) if len > 0.0 => {
                        let t = (along / len).clamp(0.0, 1.0);
                        r.forearm + (r.wrist - r.forearm) * t
                    }
                    _ => r.wrist,
                }
            }
            Kind::Other => 1.0,
        }
    };
    for (p, inf) in mesh.points.iter_mut().zip(&mesh.influences) {
        let total: f32 = inf.iter().map(|&(_, w)| w).sum();
        if total <= 0.0 {
            continue;
        }
        let mut moved = [0.0f32; 3];
        for &(b, w) in inf {
            let b = b as usize;
            let Some(Some((axis, _))) = axes.get(b) else { continue };
            let d = sub(*p, pos[b]);
            let along = dot(d, *axis);
            let radial = [d[0] - along * axis[0], d[1] - along * axis[1], d[2] - along * axis[2]];
            let s = factor(b, along) - 1.0;
            for k in 0..3 {
                moved[k] += radial[k] * s * (w / total);
            }
        }
        for k in 0..3 {
            p[k] += moved[k];
        }
    }
}

/// The nails of an arms texture: pinker and lighter than the skin around them, in blobs (thin
/// lines of the drawing are left out).
fn nail_mask(w: u32, h: u32, rgba: &[u8]) -> Vec<bool> {
    let raw: Vec<bool> = rgba.chunks_exact(4).map(|c| c[0] > 200 && (c[2] as f32) > 0.665 * c[0] as f32).collect();
    let (w, h) = (w as i64, h as i64);
    (0..w * h)
        .map(|i| {
            if !raw[i as usize] {
                return false;
            }
            let (x, y) = (i % w, i / w);
            let mut near = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if (dx, dy) != (0, 0) && nx >= 0 && ny >= 0 && nx < w && ny < h && raw[(ny * w + nx) as usize] {
                        near += 1;
                    }
                }
            }
            near >= 5
        })
        .collect()
}

/// What a texel of the arms texture shows under a sleeve.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Cloth {
    None,
    Fabric,
    Band,
    /// A thin crease line, as the game's ink.
    Ink,
}

/// An arms texture with a sleeve is made at this many times its size.
const SLEEVE_SCALE: u32 = 1;

/// The texture of a sleeve in relief (plain, `relief_texture = "painted"`): its own image, laid
/// out along the forearm (u, from SLEEVE_T0 to SLEEVE_T1 of the forearm's length) and around it
/// (v, one turn), so that its band and crease lines are drawn sharp at their exact place.
const SLEEVE_T0: f32 = -0.8;
const SLEEVE_T1: f32 = 1.1;
const SLEEVE_TEXTURE: (u32, u32) = (1024, 256);

/// Draws that texture: the fabric, the band between `band` (from, to) along the forearm, and the
/// crease lines; 4 samples per texel for smooth edges. `seam` turns it as the shell's points.
fn sleeve_texture(fabric: [f32; 3], band: Option<([f32; 3], f32, f32)>, s: &Sleeve, seam: f32) -> (u32, u32, Vec<u8>) {
    const INK: [f32; 3] = [20.0, 20.0, 22.0];
    use std::f32::consts::{PI, TAU};
    let (w, h) = SLEEVE_TEXTURE;
    let mut out = vec![255u8; (w * h * 4) as usize];
    const SUB: [f32; 2] = [0.25, 0.75];
    // What depends only on the place along the forearm (column and sample).
    struct Col {
        ring: bool,
        in_band: bool,
        check: bool,
        rib: bool,
        tab: Option<bool>,
        buttons: Vec<f32>,
        creases: [f32; CREASES.len()],
    }
    // What depends only on the angle around it (row and sample).
    struct Row {
        check: bool,
        rib: bool,
        tab: Option<bool>,
        button: f32,
        creases: [Option<(f32, f32)>; CREASES.len()],
    }
    let cols: Vec<Col> = (0..w)
        .flat_map(|x| SUB.map(|sx| (x, sx)))
        .map(|(x, sx)| {
            let t = SLEEVE_T0 + (x as f32 + sx) / w as f32 * (SLEEVE_T1 - SLEEVE_T0);
            Col {
                ring: s.rings.iter().any(|r| (t - r).abs() < s.ring_colour.map_or(0.006, |(_, w)| w / 2.0)),
                in_band: band.is_some_and(|(_, lo, hi)| t >= lo && t < hi),
                check: s.checks.is_some_and(|(_, [spacing, _, width])| (t / spacing.max(1e-3)).rem_euclid(1.0) < width),
                rib: s.ribs.is_some_and(|(_, [from, ..])| t >= from),
                tab: s.tab.and_then(|[lo, hi, ..]| (t >= lo && t <= hi).then(|| (t - lo).min(hi - t) < 0.008)),
                buttons: s.buttons.map_or(Vec::new(), |(_, [at, count, _, spacing])| (0..count as usize).map(|i| ((t - (at - i as f32 * spacing)) / 0.011).powi(2)).collect()),
                creases: CREASES.map(|(at, ..)| t - at),
            }
        })
        .collect();
    let rows: Vec<Row> = (0..h)
        .flat_map(|y| SUB.map(|sy| (y, sy)))
        .map(|(y, sy)| {
            let ang = (y as f32 + sy) / h as f32 * TAU - PI + seam * TAU;
            let da_to = |centre: f32| (ang - centre + PI).rem_euclid(TAU) - PI;
            Row {
                check: s.checks.is_some_and(|(_, [_, count, width])| ((ang + PI) / TAU * count).rem_euclid(1.0) < width),
                rib: s.ribs.is_some_and(|(_, [_, count, width])| ((ang + PI) / TAU * count).rem_euclid(1.0) < width),
                tab: s.tab.and_then(|[_, _, centre, half]| {
                    let da = da_to(centre);
                    (da.abs() <= half).then(|| half - da.abs() < 0.04)
                }),
                button: s.buttons.map_or(0.0, |(_, [_, _, centre, _])| (da_to(centre) / 0.075).powi(2)),
                creases: CREASES.map(|(_, centre, half)| {
                    let da = da_to(centre);
                    let k = 1.0 - (da / half).powi(2);
                    (da.abs() < half).then(|| (0.03 * da, 0.007 * k.max(0.0).sqrt()))
                }),
            }
        })
        .collect();
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 3];
            for (ix, iy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (col, row) = (&cols[x as usize * 2 + ix], &rows[y as usize * 2 + iy]);
                let crease = s.creases && col.creases.iter().zip(&row.creases).any(|(dt, r)| r.is_some_and(|(lean, half)| (dt - lean).abs() < half));
                let ink = crease || (col.ring && s.ring_colour.is_none());
                let mut c = match band {
                    _ if ink => INK,
                    Some((c, ..)) if col.in_band => c,
                    _ => fabric,
                };
                if let (true, Some((colour, _))) = (col.ring, s.ring_colour) {
                    c = colour.map(f32::from);
                }
                if let Some((colour, _)) = s.checks {
                    if !ink && (col.check || row.check) {
                        c = colour.map(f32::from);
                    }
                }
                if let Some((colour, _)) = s.ribs {
                    if !ink && col.rib && row.rib {
                        c = colour.map(f32::from);
                    }
                }
                if let Some((colour, _)) = s.buttons {
                    for dt2 in &col.buttons {
                        let r = (dt2 + row.button).sqrt();
                        if r < 1.0 {
                            c = if r > 0.72 { INK } else { colour.map(f32::from) };
                        }
                    }
                }
                if let (Some(along_edge), Some(around_edge)) = (col.tab, row.tab) {
                    c = if along_edge || around_edge { INK } else { fabric };
                }
                for k in 0..3 {
                    sum[k] += c[k] / 4.0;
                }
            }
            let i = ((y * w + x) * 4) as usize;
            for k in 0..3 {
                out[i + k] = sum[k].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    (w, h, out)
}

/// The cloth of an arms texture for a sleeve ending at `cuff`.
fn cloth_key(path: &str, cuff: f32) -> String {
    format!("{path}@{cuff}")
}

/// True on a crease line (CREASES) at `t` along the forearm and angle `ang` around it.
fn crease_at(t: f32, ang: f32) -> bool {
    CREASES.iter().any(|&(at, centre, half)| {
        let da = (ang - centre + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        // Thinner towards its ends, slanted along the forearm.
        let k = 1.0 - (da / half).powi(2);
        da.abs() < half && (t - at - 0.03 * da).abs() < 0.007 * k.max(0.0).sqrt()
    })
}

/// An RGBA image `k` times larger, bilinear (texel centres kept, edges clamped).
fn upscale(w: u32, h: u32, rgba: &[u8], k: u32) -> (u32, u32, Vec<u8>) {
    let (nw, nh) = (w * k, h * k);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    let at = |x: i64, y: i64, c: usize| rgba[((y.clamp(0, h as i64 - 1) * w as i64 + x.clamp(0, w as i64 - 1)) * 4) as usize + c] as f32;
    for y in 0..nh {
        for x in 0..nw {
            let fx = (x as f32 + 0.5) / k as f32 - 0.5;
            let fy = (y as f32 + 0.5) / k as f32 - 0.5;
            let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
            let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
            for c in 0..4 {
                let top = at(x0, y0, c) * (1.0 - tx) + at(x0 + 1, y0, c) * tx;
                let bottom = at(x0, y0 + 1, c) * (1.0 - tx) + at(x0 + 1, y0 + 1, c) * tx;
                out[((y * nw + x) * 4) as usize + c] = (top * (1.0 - ty) + bottom * ty).round() as u8;
            }
        }
    }
    (nw, nh, out)
}

/// The crease lines drawn on a sleeve with `creases`: (along the forearm, angle around it in
/// radians, half its length as an angle). Thin, short and slightly slanted, near the wrist and
/// near the elbow, where cloth folds.
const CREASES: [(f32, f32, f32); 2] = [(0.86, 0.6, 0.55), (0.2, 2.6, 0.45)];

/// Places every texel of texture `texture` (w × h) along the forearm: for each mesh, each face
/// drawn with it is rasterised in texture space with, at its corners, the position of its points
/// along the forearm bone that holds them (0 elbow, 1 wrist; a point of the hand or a finger is
/// past the wrist). The meshes' values are averaged; texels of no face are None.
fn along_forearm(meshes: &[(&SkeletalMesh, &[String], usize)], w: u32, h: u32) -> Vec<Option<f32>> {
    along_and_around(meshes, w, h).0
}

/// [`along_forearm`], and the angle of each texel around its forearm (from the bone's own axes, as
/// the sleeve shell; None for texels of the hands).
fn along_and_around(meshes: &[(&SkeletalMesh, &[String], usize)], w: u32, h: u32) -> (Vec<Option<f32>>, Vec<Option<f32>>) {
    let mut sum = vec![0.0f32; (w * h) as usize];
    let mut count = vec![0u32; (w * h) as usize];
    let mut cs = vec![[0.0f32; 2]; (w * h) as usize];
    struct Tri {
        a: (f32, f32, f32),
        b: (f32, f32, f32),
        c: (f32, f32, f32),
        den: f32,
        trig: Option<([f32; 3], [f32; 3])>,
        x0: i64,
        x1: i64,
        y0: i64,
        y1: i64,
    }
    let mut tris: Vec<Tri> = Vec::new();
    for &(mesh, bones, texture) in meshes {
        let bases = mesh.ref_space_bases();
        let pos: Vec<[f32; 3]> = bases.iter().map(|b| b.origin).collect();
        let kinds: Vec<Kind> = bones.iter().map(|b| kind(b)).collect();
        // Each forearm bone: its position and the vector to its hand.
        let forearm: Vec<Option<([f32; 3], [f32; 3])>> = (0..mesh.bones.len())
            .map(|b| {
                if kinds[b] != Kind::Forearm {
                    return None;
                }
                // Its hand: the nearest hand bone of the same side (FPS_ right, FPSg_ left), a
                // child or not.
                let side = |n: &str| n.to_ascii_lowercase().starts_with("fpsg");
                let hand = (0..mesh.bones.len())
                    .filter(|&c| kinds[c] == Kind::Hand && side(&bones[c]) == side(&bones[b]))
                    .min_by(|&x, &y| {
                        let (dx, dy) = (sub(pos[x], pos[b]), sub(pos[y], pos[b]));
                        dot(dx, dx).total_cmp(&dot(dy, dy))
                    })?;
                Some((pos[b], sub(pos[hand], pos[b])))
            })
            .collect();
        let t_point: Vec<Option<f32>> = mesh
            .points
            .iter()
            .zip(&mesh.influences)
            .map(|(p, inf)| {
                let (mut s, mut tw) = (0.0f32, 0.0f32);
                for &(b, wt) in inf {
                    let b = b as usize;
                    let t = match kinds.get(b) {
                        Some(Kind::Forearm) => {
                            let (o, d) = forearm[b]?;
                            dot(sub(*p, o), d) / dot(d, d).max(1e-6)
                        }
                        // Past the wrist: the hand with the first bone of each finger (they hold
                        // the back of the hand up to the knuckles), then the rest of the fingers
                        // further (fingerless gloves).
                        Some(Kind::Hand) => 1.5,
                        Some(Kind::Finger) if bones[b].ends_with("01") => 1.5,
                        Some(Kind::Finger) => 2.0,
                        _ => continue,
                    };
                    s += t * wt;
                    tw += wt;
                }
                (tw > 0.0).then(|| s / tw)
            })
            .collect();
        // The angle around the forearm of the points it holds most.
        let angle: Vec<Option<f32>> = mesh
            .points
            .iter()
            .zip(&mesh.influences)
            .map(|(p, inf)| {
                let b = main_bone(inf)?;
                let (o, d) = forearm.get(b).copied().flatten()?;
                let (e1, e2) = forearm_frame(&bases[b], d);
                let v = sub(*p, o);
                Some(dot(v, e2).datan2(dot(v, e1)))
            })
            .collect();
        let tex_of = face_textures(mesh);
        for (f, face) in mesh.faces.iter().enumerate() {
            if tex_of[f] != texture {
                continue;
            }
            let corners = face.wedges.map(|k| {
                let wd = &mesh.wedges[k as usize];
                (wd.uv[0] * w as f32, wd.uv[1] * h as f32, t_point[wd.point as usize])
            });
            let angles = face.wedges.map(|k| angle[mesh.wedges[k as usize].point as usize]);
            let trig = match angles {
                [Some(x0), Some(x1), Some(x2)] => Some(([x0.dcos(), x1.dcos(), x2.dcos()], [x0.dsin(), x1.dsin(), x2.dsin()])),
                _ => None,
            };
            if corners.iter().any(|c| c.2.is_none()) {
                continue;
            }
            let (a, b, c) = (corners[0], corners[1], corners[2]);
            let den = (b.1 - c.1) * (a.0 - c.0) + (c.0 - b.0) * (a.1 - c.1);
            if den.abs() < 1e-9 {
                continue;
            }
            let x0 = a.0.min(b.0).min(c.0).floor().max(0.0) as i64;
            let x1 = (a.0.max(b.0).max(c.0).ceil() as i64).min(w as i64 - 1);
            let y0 = a.1.min(b.1).min(c.1).floor().max(0.0) as i64;
            let y1 = (a.1.max(b.1).max(c.1).ceil() as i64).min(h as i64 - 1);
            tris.push(Tri { a: (a.0, a.1, a.2.unwrap()), b: (b.0, b.1, b.2.unwrap()), c: (c.0, c.1, c.2.unwrap()), den, trig, x0, x1, y0, y1 });
        }
    }
    // The faces drawn in their order on each band of rows, one band per thread: every texel gets
    // the same sums in the same order as on one thread.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(4);
    let band = (h as usize).div_ceil(threads).max(1);
    let draw = |first: usize, sum: &mut [f32], count: &mut [u32], cs: &mut [[f32; 2]]| {
        let rows = (sum.len() / w as usize) as i64;
        let (lo, hi) = (first as i64, first as i64 + rows - 1);
        for t in &tris {
            let (a, b, c, den) = (t.a, t.b, t.c, t.den);
            for y in t.y0.max(lo)..=t.y1.min(hi) {
                for x in t.x0..=t.x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let l1 = ((b.1 - c.1) * (px - c.0) + (c.0 - b.0) * (py - c.1)) / den;
                    let l2 = ((c.1 - a.1) * (px - c.0) + (a.0 - c.0) * (py - c.1)) / den;
                    let l3 = 1.0 - l1 - l2;
                    if l1 < -0.02 || l2 < -0.02 || l3 < -0.02 {
                        continue;
                    }
                    let i = ((y - lo) * w as i64 + x) as usize;
                    sum[i] += l1 * a.2 + l2 * b.2 + l3 * c.2;
                    count[i] += 1;
                    // Angles interpolated as directions (no jump across +-pi).
                    if let Some((co, si)) = t.trig {
                        cs[i][0] += l1 * co[0] + l2 * co[1] + l3 * co[2];
                        cs[i][1] += l1 * si[0] + l2 * si[1] + l3 * si[2];
                    }
                }
            }
        }
    };
    let chunk = band * w as usize;
    let priority = openxiii_package::background::current();
    std::thread::scope(|sc| {
        let draw = &draw;
        for (n, ((s, k), d)) in sum.chunks_mut(chunk).zip(count.chunks_mut(chunk)).zip(cs.chunks_mut(chunk)).enumerate() {
            if threads == 1 {
                draw(n * band, s, k, d);
            } else {
                let math = openxiii_math::Mode::current();
                sc.spawn(move || {
                    let _math = math.enter();
                    openxiii_package::background::take(priority);
                    draw(n * band, s, k, d)
                });
            }
        }
    });
    let along = sum.iter().zip(&count).map(|(&s, &n)| (n > 0).then(|| s / n as f32)).collect();
    let around = cs.iter().map(|c| (c[0] * c[0] + c[1] * c[1] > 1e-6).then(|| c[1].datan2(c[0]))).collect();
    (along, around)
}

/// A frame around a forearm (unit e1, e2 perpendicular to `d`), from the bone's own axes: the
/// same in every mesh of the rig.
fn forearm_frame(base: &crate::coords::Coords, d: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let l = dot(d, d).sqrt().max(1e-6);
    let a = d.map(|c| c / l);
    let axes = [base.x, base.y, base.z];
    let pick = axes.iter().min_by(|p, q| dot(**p, a).abs().total_cmp(&dot(**q, a).abs())).copied().unwrap();
    let along = dot(pick, a);
    let v = [pick[0] - a[0] * along, pick[1] - a[1] * along, pick[2] - a[2] * along];
    let lv = dot(v, v).sqrt().max(1e-6);
    let e1 = v.map(|c| c / lv);
    let e2 = [a[1] * e1[2] - a[2] * e1[1], a[2] * e1[0] - a[0] * e1[2], a[0] * e1[1] - a[1] * e1[0]];
    (e1, e2)
}

/// The cloth of each texel for a sleeve ending at `cuff` with a band of `band` (along the
/// forearm). Texels of no face next to cloth take it too (a few texels), so that the texture's
/// smaller mips do not bleed the skin into the sleeve's edges.
fn cloth_mask(along: &[Option<f32>], w: u32, h: u32, cuff: f32, band: f32) -> Vec<Cloth> {
    cloth_mask_with(along, &[], w, h, cuff, band, None, false)
}

/// [`cloth_mask`] with the band at `band_at` (its start along the forearm) instead of at the
/// cuff, and the crease lines when `creases` (they need the texels' angles, `around`).
#[allow(clippy::too_many_arguments)]
fn cloth_mask_with(along: &[Option<f32>], around: &[Option<f32>], w: u32, h: u32, cuff: f32, band: f32, band_at: Option<f32>, creases: bool) -> Vec<Cloth> {
    let (band_lo, band_hi) = match band_at {
        Some(a) => (a, a + band),
        None => (cuff - band, cuff),
    };
    let crease = |t: f32, ang: Option<f32>| ang.is_some_and(|a| crease_at(t, a));
    let mut out: Vec<Cloth> = along
        .iter()
        .enumerate()
        .map(|(i, t)| match t {
            Some(t) if *t >= cuff => Cloth::None,
            Some(t) if *t >= band_lo && *t < band_hi => Cloth::Band,
            Some(t) if creases && crease(*t, around.get(i).copied().flatten()) => Cloth::Ink,
            Some(_) => Cloth::Fabric,
            None => Cloth::None,
        })
        .collect();
    let (w, h) = (w as i64, h as i64);
    for _ in 0..4 {
        let prev = out.clone();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if along[i].is_some() || prev[i] != Cloth::None {
                    continue;
                }
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < w && ny < h && prev[(ny * w + nx) as usize] != Cloth::None {
                        out[i] = prev[(ny * w + nx) as usize];
                        break;
                    }
                }
            }
        }
    }
    out
}

/// Recolours the arms texture: every texel multiplied by target / median of the texture (the
/// drawing and shading kept), the nails to their own colour with the same shading, and the
/// sleeve's texels (`cloth`) to the fabric's or the band's colour with the same shading.
fn recolour(w: u32, h: u32, rgba: &mut [u8], target: Option<[f32; 3]>, nails: Option<[u8; 3]>) -> (usize, [f32; 3]) {
    recolour_with(w, h, rgba, target, nails, None, None)
}

fn recolour_with(
    w: u32,
    h: u32,
    rgba: &mut [u8],
    target: Option<[f32; 3]>,
    nails: Option<[u8; 3]>,
    cloth: Option<(&[Cloth], [f32; 3], [f32; 3])>,
    gloves: Option<(&[bool], [f32; 3])>,
) -> (usize, [f32; 3]) {
    let mask = nail_mask(w, h, rgba);
    // The nails and a few texels around them (their rim), erased under gloves.
    let near_nail: Vec<bool> = if gloves.is_none() {
        Vec::new()
    } else {
        // A 9 x 9 window, as two passes (rows, then columns): the same texels.
        let (wi, hi) = (w as usize, h as usize);
        let mut rows = vec![false; wi * hi];
        for y in 0..hi {
            let row = &mask[y * wi..(y + 1) * wi];
            for x in 0..wi {
                rows[y * wi + x] = row[x.saturating_sub(4)..(x + 5).min(wi)].iter().any(|&m| m);
            }
        }
        let mut out = vec![false; wi * hi];
        for y in 0..hi {
            for yy in y.saturating_sub(4)..(y + 5).min(hi) {
                for x in 0..wi {
                    out[y * wi + x] |= rows[yy * wi + x];
                }
            }
        }
        out
    };
    let skin: Vec<[u8; 3]> = rgba.chunks_exact(4).zip(&mask).filter(|(_, &m)| !m).map(|(c, _)| [c[0], c[1], c[2]]).collect();
    let base = if skin.is_empty() { [1.0; 3] } else { median(&skin) };
    let base_lum = (base[0] + base[1] + base[2]) / 3.0;
    let lum = |c: &[u8]| (c[0] as f32 + c[1] as f32 + c[2] as f32) / 3.0;
    let nail_lum = {
        let v: Vec<f32> = rgba.chunks_exact(4).zip(&mask).filter(|(_, &m)| m).map(|(c, _)| lum(c)).collect();
        if v.is_empty() { 1.0 } else { v.iter().sum::<f32>() / v.len() as f32 }
    };
    for (i, (c, &m)) in rgba.chunks_exact_mut(4).zip(&mask).enumerate() {
        if let Some((cl, fabric, band)) = cloth {
            let colour = match cl[i] {
                Cloth::Fabric => Some(fabric),
                Cloth::Band => Some(band),
                // The game's ink: near black.
                Cloth::Ink => Some([20.0, 20.0, 22.0]),
                Cloth::None => None,
            };
            if let Some(col) = colour {
                // Plain cloth: the skin's drawing (muscles, veins) does not belong on a sleeve; its
                // shadows come from the game's cartoon shading.
                for j in 0..3 {
                    c[j] = col[j].round().clamp(0.0, 255.0) as u8;
                }
                continue;
            }
        }
        // Gloves: plain leather. Of the hands' drawing only its dark lines stay (between the
        // fingers, the knuckles' folds), as one darker tone; the skin's lighter details and the
        // nails (and their rim) are the gloves' colour.
        if let Some((g, colour)) = gloves {
            if g[i] {
                let ratio = lum(c) / base_lum.max(1.0);
                let tone = if ratio < 0.72 && !near_nail[i] { 0.62 } else { 1.0 };
                for k in 0..3 {
                    c[k] = (colour[k] * tone).round().clamp(0.0, 255.0) as u8;
                }
                continue;
            }
        }
        let new = match (m, nails, target) {
            (true, Some(n), _) => {
                let k = lum(c) / nail_lum.max(1.0);
                n.map(|v| v as f32 * k)
            }
            (_, _, Some(t)) => [0, 1, 2].map(|k| c[k] as f32 * t[k] / base[k].max(1.0)),
            _ => continue,
        };
        for k in 0..3 {
            c[k] = new[k].round().clamp(0.0, 255.0) as u8;
        }
    }
    (mask.iter().filter(|&&m| m).count(), base)
}

/// Where a character model's forearm sleeve lies in its texture, for each side (right, left):
/// its texture, and texture coordinates = origin + along · t + around · s, with t from the elbow
/// (0) to the wrist (1) and s around the arm (0 to 1). Fitted on the faces held by the forearm
/// bone: `along` by least squares on t, `around` across the rest of the island.
type SleeveUv = (String, [f32; 2], [f32; 2], [f32; 2]);

fn sleeve_uv(library: &mut Library, model: &str) -> Result<[SleeveUv; 2], String> {
    let (mesh, bones, textures) = read_character(library, model)?;
    let bases = mesh.ref_space_bases();
    let pos: Vec<[f32; 3]> = bases.iter().map(|b| b.origin).collect();
    let lower: Vec<String> = bones.iter().map(|b| format!(" {} ", b.to_ascii_lowercase())).collect();
    let tex_of = face_textures(&mesh);
    let mut out = Vec::new();
    for side in [" r ", " l "] {
        let fa = lower.iter().position(|n| n.contains("forearm") && n.contains(side)).ok_or_else(|| format!("{model}: no{side}forearm bone"))?;
        let hand = lower.iter().position(|n| n.contains("hand") && n.contains(side)).ok_or_else(|| format!("{model}: no{side}hand bone"))?;
        let d = sub(pos[hand], pos[fa]);
        let dd = dot(d, d).max(1e-6);
        let mut samples: Vec<(f32, [f32; 2])> = Vec::new();
        let mut counts: HashMap<usize, usize> = HashMap::new();
        for (f, face) in mesh.faces.iter().enumerate() {
            let pts = face.wedges.map(|w| mesh.wedges[w as usize].point as usize);
            if !pts.iter().all(|&p| main_bone(&mesh.influences[p]) == Some(fa)) {
                continue;
            }
            *counts.entry(tex_of[f]).or_default() += 1;
            for w in face.wedges {
                let wd = &mesh.wedges[w as usize];
                samples.push((dot(sub(mesh.points[wd.point as usize], pos[fa]), d) / dd, wd.uv));
            }
        }
        if samples.len() < 6 {
            return Err(format!("{model}: too few faces on the{side}forearm"));
        }
        let texture = counts.into_iter().max_by_key(|&(_, c)| c).map(|(t, _)| textures[t].clone()).unwrap_or_default();
        let (origin, along, around, ..) = fit_island(samples).ok_or_else(|| format!("{model}: the{side}forearm's sleeve is not one piece of its texture"))?;
        out.push((texture, origin, along, around));
    }
    let l = out.pop().unwrap();
    let r = out.pop().unwrap();
    Ok([r, l])
}

/// The sleeve's own texture made from a character model's sleeve (its printed cloth, a
/// camouflage): only the lit half of the island around the arm is kept (the other half carries
/// the painted shadow of the underside), mirrored all around the sleeve so that its edges meet,
/// and mirrored past the elbow and the wrist the same way; the game's shading makes the shadows.
fn model_lit_texture(library: &mut Library, uv: &SleeveUv) -> Result<(u32, u32, Vec<u8>), String> {
    let (texture, origin, along, around) = uv;
    let (tw, th, img) = read_texture(library, texture)?;
    let at = |t: f32, a: f32| -> [f32; 3] {
        let u = (origin[0] + along[0] * t + around[0] * a).rem_euclid(1.0) * tw as f32 - 0.5;
        let v = (origin[1] + along[1] * t + around[1] * a).rem_euclid(1.0) * th as f32 - 0.5;
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let px = |x: f32, y: f32, c: usize| img[(((y as i64).clamp(0, th as i64 - 1) * tw as i64 + (x as i64).clamp(0, tw as i64 - 1)) * 4) as usize + c] as f32;
        [0, 1, 2].map(|c| (px(x0, y0, c) * (1.0 - fx) + px(x0 + 1.0, y0, c) * fx) * (1.0 - fy) + (px(x0, y0 + 1.0, c) * (1.0 - fx) + px(x0 + 1.0, y0 + 1.0, c) * fx) * fy)
    };
    // The lit half: the half-turn window brightest on average.
    const BINS: usize = 32;
    let lum: Vec<f32> = (0..BINS)
        .map(|i| (0..16).map(|k| at(0.1 + 0.8 * k as f32 / 15.0, (i as f32 + 0.5) / BINS as f32).iter().sum::<f32>()).sum::<f32>())
        .collect();
    let start = (0..BINS).max_by(|&x, &y| {
        let w = |s: usize| (0..BINS / 2).map(|i| lum[(s + i) % BINS]).sum::<f32>();
        w(x).total_cmp(&w(y))
    }).unwrap_or(0) as f32 / BINS as f32;
    let ping = |x: f32| {
        let m = x.rem_euclid(2.0);
        if m > 1.0 { 2.0 - m } else { m }
    };
    let (w, h) = SLEEVE_TEXTURE;
    let mut out = vec![255u8; (w * h * 4) as usize];
    for y in 0..h {
        let a = start + 0.5 * ping(2.0 * (y as f32 + 0.5) / h as f32);
        for x in 0..w {
            let t = ping(SLEEVE_T0 + (x as f32 + 0.5) / w as f32 * (SLEEVE_T1 - SLEEVE_T0));
            let c = at(t.clamp(0.02, 0.98), a);
            let i = ((y * w + x) * 4) as usize;
            for k in 0..3 {
                out[i + k] = c[k].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Ok((w, h, out))
}

/// The texture coordinates of a forearm's island from samples (t along the forearm, uv): origin,
/// along (per unit of t) and around (its width across). Only the main island (samples near the
/// median; a few faces may be drawn elsewhere in the texture).
fn fit_island(mut samples: Vec<(f32, [f32; 2])>) -> Option<([f32; 2], [f32; 2], [f32; 2], f32, f32)> {
    if samples.len() < 6 {
        return None;
    }
    let med = [0, 1].map(|k| {
        let mut v: Vec<f32> = samples.iter().map(|s| s.1[k]).collect();
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    });
    samples.retain(|s| (s.1[0] - med[0]).abs() < 0.15 && (s.1[1] - med[1]).abs() < 0.15);
    if samples.len() < 6 {
        return None;
    }
    // u and v as lines of t.
    let n = samples.len() as f32;
    let mt = samples.iter().map(|s| s.0).sum::<f32>() / n;
    let mu = [0, 1].map(|k| samples.iter().map(|s| s.1[k]).sum::<f32>() / n);
    let vt = samples.iter().map(|s| (s.0 - mt) * (s.0 - mt)).sum::<f32>().max(1e-9);
    let along = [0, 1].map(|k| samples.iter().map(|s| (s.0 - mt) * (s.1[k] - mu[k])).sum::<f32>() / vt);
    let base = [0, 1].map(|k| mu[k] - along[k] * mt);
    let len = (along[0] * along[0] + along[1] * along[1]).sqrt().max(1e-9);
    let perp = [-along[1] / len, along[0] / len];
    let across: Vec<f32> = samples.iter().map(|s| (s.1[0] - base[0] - along[0] * s.0) * perp[0] + (s.1[1] - base[1] - along[1] * s.0) * perp[1]).collect();
    let lo = across.iter().cloned().fold(f32::MAX, f32::min);
    let hi = across.iter().cloned().fold(f32::MIN, f32::max);
    let t_lo = samples.iter().map(|s| s.0).fold(f32::MAX, f32::min);
    let t_hi = samples.iter().map(|s| s.0).fold(f32::MIN, f32::max);
    Some(([base[0] + perp[0] * lo, base[1] + perp[1] * lo], along, [perp[0] * (hi - lo), perp[1] * (hi - lo)], t_lo, t_hi))
}

/// Adds a sleeve shell around each forearm of a first-person mesh (reference pose): rings of
/// points around the forearm bone's axis, from the arm's elbow end to the cuff, each sector at the
/// arm's own radius there times `thickness` plus `room`; at the cuff a lip (`lip` wider) folding
/// back inside. Skinned on the forearm bone only (it follows the forearm, not the wrist). Its
/// faces use part `part`, with the texture coordinates of `uv` (right, left).
fn add_sleeve_shell(mesh: &mut SkeletalMesh, bones: &[String], cuff: f32, m: &SleeveModel, uv: Option<&[SleeveUv; 2]>, parts: [u16; 2], hidden: Option<Side>) -> usize {
    use crate::skeletal::{Face, Wedge};
    const SECTORS: usize = 24;
    const RINGS: usize = 28;
    let bases = mesh.ref_space_bases();
    let pos: Vec<[f32; 3]> = bases.iter().map(|b| b.origin).collect();
    let kinds: Vec<Kind> = bones.iter().map(|b| kind(b)).collect();
    let left = |n: &str| n.to_ascii_lowercase().starts_with("fpsg");
    let mut added = 0;
    for b in 0..mesh.bones.len() {
        if kinds[b] != Kind::Forearm || hidden.is_some_and(|h| (h == Side::Left) == left(&bones[b])) {
            continue;
        }
        let Some(hand) = (0..mesh.bones.len()).filter(|&c| kinds[c] == Kind::Hand && left(&bones[c]) == left(&bones[b])).min_by(|&x, &y| {
            let (dx, dy) = (sub(pos[x], pos[b]), sub(pos[y], pos[b]));
            dot(dx, dx).total_cmp(&dot(dy, dy))
        }) else {
            continue;
        };
        let d = sub(pos[hand], pos[b]);
        let len = dot(d, d).sqrt();
        if len < 1e-3 {
            continue;
        }
        let a = d.map(|c| c / len);
        // A frame around the axis, from the bone's own axes (the same in every mesh of the rig).
        let axes = [bases[b].x, bases[b].y, bases[b].z];
        let pick = axes.iter().min_by(|p, q| dot(**p, a).abs().total_cmp(&dot(**q, a).abs())).copied().unwrap();
        let along_pick = dot(pick, a);
        let e1 = {
            let v = [pick[0] - a[0] * along_pick, pick[1] - a[1] * along_pick, pick[2] - a[2] * along_pick];
            let l = dot(v, v).sqrt().max(1e-6);
            v.map(|c| c / l)
        };
        let e2 = [a[1] * e1[2] - a[2] * e1[1], a[2] * e1[0] - a[0] * e1[2], a[0] * e1[1] - a[1] * e1[0]];
        // The arm's points held by this forearm (any weight) or by its hand near the wrist:
        // (t, angle, radius).
        let mut arm: Vec<(f32, f32, f32)> = Vec::new();
        for (p, inf) in mesh.points.iter().zip(&mesh.influences) {
            let held = inf.iter().any(|&(i, w)| i as usize == b && w > 0.05);
            let main = main_bone(inf);
            let near_hand = main.is_some_and(|i| kinds.get(i) == Some(&Kind::Hand) && left(&bones[i]) == left(&bones[b]));
            if !held && !near_hand {
                continue;
            }
            let v = sub(*p, pos[b]);
            let t = dot(v, a) / len;
            if near_hand && !held && t > cuff + 0.06 {
                continue;
            }
            let x = dot(v, e1);
            let y = dot(v, e2);
            arm.push((t, y.datan2(x), (x * x + y * y).sqrt()));
        }
        if arm.len() < 8 {
            continue;
        }
        let t0 = arm.iter().map(|p| p.0).fold(f32::MAX, f32::min).max(-0.5);
        if t0 >= cuff - 0.05 {
            continue;
        }
        let ring_t: Vec<f32> = (0..RINGS).map(|k| t0 + (cuff - 0.03 - t0) * k as f32 / (RINGS - 1) as f32).collect();
        let step = (cuff - t0) / (RINGS - 1) as f32;
        let sector = std::f32::consts::TAU / SECTORS as f32;
        let mut radius = vec![[0.0f32; SECTORS]; RINGS];
        for (k, &t) in ring_t.iter().enumerate() {
            for s in 0..SECTORS {
                let ang = s as f32 * sector - std::f32::consts::PI + m.seam * std::f32::consts::TAU;
                let near = |p: &&(f32, f32, f32)| {
                    // The cheap test first (same result: both must hold).
                    (p.0 - t).abs() <= step * 0.75 + 0.04 && {
                        let da = (p.1 - ang + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                        da.abs() <= sector * 1.6
                    }
                };
                radius[k][s] = arm.iter().filter(near).map(|p| p.2).fold(0.0, f32::max);
            }
            // Empty sectors: the ring's largest radius.
            let most = radius[k].iter().cloned().fold(0.0, f32::max);
            for r in radius[k].iter_mut().filter(|r| **r <= 0.0) {
                *r = most;
            }
        }
        // Smooth around and along, never under the arm.
        let raw = radius.clone();
        for k in 0..RINGS {
            for s in 0..SECTORS {
                let around = (raw[k][(s + SECTORS - 1) % SECTORS] + 2.0 * raw[k][s] + raw[k][(s + 1) % SECTORS]) / 4.0;
                let next = if k + 1 < RINGS { raw[k + 1][s] } else { raw[k][s] };
                radius[k][s] = around.max(raw[k][s]).max(next * 0.9);
            }
        }
        // Straight, as a fitted uniform's sleeve: each sector's radius becomes a straight line from
        // the elbow to the cuff (least squares on its rings), raised just enough to stay above the
        // arm everywhere; no bulge where the arm itself bulges.
        for s in 0..SECTORS {
            let n = RINGS as f32;
            let mt = ring_t.iter().sum::<f32>() / n;
            let mr = (0..RINGS).map(|k| radius[k][s]).sum::<f32>() / n;
            let vt = ring_t.iter().map(|t| (t - mt) * (t - mt)).sum::<f32>().max(1e-9);
            let slope = (0..RINGS).map(|k| (ring_t[k] - mt) * (radius[k][s] - mr)).sum::<f32>() / vt;
            let line = |t: f32| mr + slope * (t - mt);
            let lift = (0..RINGS).map(|k| radius[k][s] - line(ring_t[k])).fold(0.0f32, f32::max);
            for k in 0..RINGS {
                radius[k][s] = line(ring_t[k]) + lift;
            }
        }
        // Narrower towards the cuff, smoothly.
        if let Some((from, factor)) = m.taper {
            for k in 0..RINGS {
                let x = ((ring_t[k] - from) / (cuff - from).max(1e-3)).clamp(0.0, 1.0);
                let f = 1.0 + (factor - 1.0) * x * x * (3.0 - 2.0 * x);
                for r in radius[k].iter_mut() {
                    *r *= f;
                }
            }
        }
        // Folds: soft rolls around the arm, leaning (slant) and of uneven depth, spread from the
        // elbow side to just before the cuff, the first and last ones deeper.
        let fold_at = |t: f32, s: usize| -> f32 {
            let ang = s as f32 * sector;
            let first = t0.max(0.1);
            (0..m.folds)
                .map(|i| {
                    let at = first + (cuff - 0.06 - first) * (i as f32 + 0.5) / m.folds as f32;
                    let strength = if i == 0 || i + 1 == m.folds { 1.0 } else { 0.65 };
                    let lean = m.fold_slant * (ang + i as f32 * 1.7).dsin();
                    let x = (t - at - lean) / m.fold_width.max(1e-3);
                    strength * (-x * x).dexp()
                })
                .fold(0.0, f32::max)
        };
        // Rings: the shell, then the lip (out at the cuff, then back inside).
        let mut rings: Vec<(f32, [f32; SECTORS])> = Vec::new();
        // Past the elbow end, the same straight tube a little further, closed flat: no opening and
        // no ball where the arm leaves the screen.
        let first = radius[0].map(|r| r * m.thickness + m.room);
        rings.push((t0 - 0.2, first.map(|r| r * 0.02)));
        rings.push((t0 - 0.2, first));
        for k in 0..RINGS {
            let mut ring = radius[k].map(|r| r * m.thickness + m.room);
            for (s, r) in ring.iter_mut().enumerate() {
                *r *= 1.0 + m.fold_depth * fold_at(ring_t[k], s);
            }
            rings.push((ring_t[k], ring));
        }
        let last = radius[RINGS - 1].map(|r| r * m.thickness + m.room);
        rings.push((cuff, last.map(|r| r * m.lip)));
        rings.push((cuff + 0.01, last.map(|r| (r * m.lip + r) * 0.5)));
        rings.push((cuff, last.map(|r| r * 0.97)));
        rings.push((cuff - 0.08, last.map(|r| r * 0.95)));
        let side = usize::from(left(&bones[b]));
        // Texture coordinates: the model's sleeve, or the sleeve's own texture.
        let (origin, along_uv, around_uv, t_min, t_max) = match uv {
            Some(uv) => (uv[side].1, uv[side].2, uv[side].3, 0.0, 1.0),
            // The sleeve's own texture (sleeve_texture): along the forearm, then around it.
            None => {
                let span = SLEEVE_T1 - SLEEVE_T0;
                ([-SLEEVE_T0 / span, 0.0], [1.0 / span, 0.0], [0.0, 1.0], SLEEVE_T0, SLEEVE_T1)
            }
        };
        let base_wedge = mesh.wedges.len();
        let base_point = mesh.points.len();
        if base_point + rings.len() * SECTORS > 65535 || base_wedge + rings.len() * (SECTORS + 1) > 65535 {
            continue;
        }
        for &(t, ref rr) in &rings {
            for (s, &r) in rr.iter().enumerate() {
                let ang = s as f32 * sector - std::f32::consts::PI + m.seam * std::f32::consts::TAU;
                let (c, sn) = (ang.dcos(), ang.dsin());
                let o = [pos[b][0] + a[0] * t * len, pos[b][1] + a[1] * t * len, pos[b][2] + a[2] * t * len];
                mesh.points.push([0, 1, 2].map(|k| o[k] + (e1[k] * c + e2[k] * sn) * r));
                mesh.influences.push(vec![(b as u16, 1.0)]);
            }
        }
        for (k, &(t, _)) in rings.iter().enumerate() {
            let tc = t.clamp(t_min, t_max);
            for s in 0..=SECTORS {
                // The sleeve's own texture is mirrored on the left arm, so that what it shows (a
                // tab, creases) sits at the same place on both arms.
                let around = if uv.is_none() && side == 1 { 1.0 - s as f32 / SECTORS as f32 - 2.0 * m.seam } else { s as f32 / SECTORS as f32 };
                let u = origin[0] + along_uv[0] * tc + around_uv[0] * around;
                let v = origin[1] + along_uv[1] * tc + around_uv[1] * around;
                mesh.wedges.push(Wedge { point: (base_point + k * SECTORS + s % SECTORS) as u16, uv: [u, v] });
            }
        }
        let w = |k: usize, s: usize| (base_wedge + k * (SECTORS + 1) + s) as u16;
        for k in 0..rings.len() - 1 {
            for s in 0..SECTORS {
                // Faces wound as the game's (clockwise seen from outside; the sector order goes e1 → e2
                // around +t).
                mesh.faces.push(Face { wedges: [w(k, s), w(k + 1, s + 1), w(k, s + 1)], material: parts[side] });
                mesh.faces.push(Face { wedges: [w(k, s), w(k + 1, s), w(k + 1, s + 1)], material: parts[side] });
            }
        }
        added += 1;
    }
    added
}

/// The meshes named by `from`: a mesh as is, "Package.*" for every skeletal mesh of the package
/// with first-person arm bones (qualified "Package.Mesh"); those of `skip` left out.
pub fn sources(library: &mut Library, from: &[String], skip: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for f in from {
        let Some(package) = f.strip_suffix(".*") else {
            out.push(f.clone());
            continue;
        };
        let loaded = library.get(package).ok_or_else(|| format!("package {package} not found in the game"))?;
        let (p, d) = (&loaded.package, &loaded.data);
        for i in 0..p.exports.len() {
            if p.export_class(i) != "SkeletalMesh" || p.exports[i].serial_size == 0 {
                continue;
            }
            let Ok(m) = SkeletalMesh::read(p, d, i) else { continue };
            if m.bones.iter().any(|b| kind(p.name(b.name)) != Kind::Other) {
                out.push(format!("{package}.{}", p.object_path(ObjectRef::Export(i))));
            }
        }
    }
    out.retain(|m| !skip.iter().any(|s| s.eq_ignore_ascii_case(m)));
    let mut seen = std::collections::HashSet::new();
    out.retain(|m| seen.insert(m.to_ascii_lowercase()));
    Ok(out)
}

/// What every recipe with the same `from` and `skip` reads and computes the same way: the list
/// of meshes, the meshes themselves, the arms textures and their texels placed along the forearm.
struct Shared {
    key: (Vec<String>, Vec<String>),
    list: Vec<String>,
    meshes: Vec<GameMesh>,
    textures: HashMap<String, (u32, u32, Vec<u8>)>,
    #[allow(clippy::type_complexity)]
    placed: Option<Vec<(String, u32, u32, Vec<Option<f32>>, Vec<Option<f32>>, usize)>>,
}

static SHARED: std::sync::Mutex<Option<Shared>> = std::sync::Mutex::new(None);

fn shared_texture(sh: &mut Shared, library: &mut Library, path: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let key = path.to_ascii_lowercase();
    if !sh.textures.contains_key(&key) {
        let t = read_texture(library, path)?;
        sh.textures.insert(key.clone(), t);
    }
    Ok(sh.textures[&key].clone())
}

/// Makes the arms `name` ("Mod.Jones") from the game's meshes by the recipe. Each skin texture
/// is recoloured once ("<name>.skin", then "<name>.skin2"…) and shared by every mesh using it.
pub fn build(library: &mut Library, name: &str, r: &Recipe) -> Result<Built, String> {
    let mut notes = Vec::new();
    let target = match &r.skin {
        Skin::Unchanged => None,
        Skin::Color(c) => Some(c.map(|v| v as f32)),
        Skin::From(model) => {
            // Under a sleeve, only the hands show the skin.
            // Under a sleeve only the hands show the skin, with gloves only the forearms.
            let t = match (&r.sleeve, &r.gloves) {
                (Some(_), _) => measure(library, model, &["hand", "finger"])?,
                (None, Some(_)) => measure(library, model, &["forearm"])?,
                (None, None) => measure_skin(library, model)?,
            };
            notes.push(format!("skin colour measured on {model}: {:.0}, {:.0}, {:.0}", t[0], t[1], t[2]));
            Some(t)
        }
    };
    let mut guard = SHARED.lock().unwrap();
    if guard.as_ref().is_some_and(|s| s.key != (r.from.clone(), r.skip.clone())) {
        *guard = None;
    }
    if guard.is_none() {
        let list = sources(library, &r.from, &r.skip)?;
        let mut meshes = Vec::with_capacity(list.len());
        for from in &list {
            meshes.push(read_mesh(library, from)?);
        }
        *guard = Some(Shared { key: (r.from.clone(), r.skip.clone()), list, meshes, textures: HashMap::new(), placed: None });
    }
    let sh = guard.as_mut().unwrap();
    let list = sh.list.clone();
    // The sleeve: its colours, and the cloth of each texel of each arms texture, placed along
    // the forearm from every mesh drawn with it.
    let mut cloth: HashMap<String, (Vec<Cloth>, [f32; 3], [f32; 3])> = HashMap::new();
    let mut sleeve_colours: Option<([f32; 3], [f32; 3])> = None;
    // Each arms texture's texels placed along (and around) the forearm, from every mesh drawn
    // with it: for a sleeve or gloves.
    let mut placed: Vec<(String, u32, u32, Vec<Option<f32>>, Vec<Option<f32>>, usize)> = Vec::new();
    if (r.sleeve.is_some() || r.gloves.is_some()) && sh.placed.is_none() {
        let mut read: Vec<(&SkeletalMesh, &[String], usize, String)> = Vec::new();
        for (mesh, bones, textures, ..) in &sh.meshes {
            if let Some(i) = arms_texture(mesh, bones, textures.len()) {
                read.push((mesh, bones.as_slice(), i, textures[i].to_ascii_lowercase()));
            }
        }
        let mut paths: Vec<String> = read.iter().map(|m| m.3.clone()).collect();
        paths.sort();
        paths.dedup();
        let mut out = Vec::new();
        for path in paths {
            let (w, h, _) = read_texture(library, &path)?;
            let (w, h) = (w * SLEEVE_SCALE, h * SLEEVE_SCALE);
            let on: Vec<(&SkeletalMesh, &[String], usize)> = read.iter().filter(|m| m.3 == path).map(|m| (m.0, m.1, m.2)).collect();
            let (along, around) = along_and_around(&on, w, h);
            out.push((path, w, h, along, around, on.len()));
        }
        sh.placed = Some(out);
    }
    if r.sleeve.is_some() || r.gloves.is_some() {
        placed = sh.placed.clone().unwrap();
    }
    if let Some(s) = &r.sleeve {
        let fabric = match &s.fabric {
            Skin::Color(c) => c.map(|v| v as f32),
            Skin::From(model) => {
                // The lit cloth: its painted shadows (darker half) left out, the light comes
                // from the game's shading.
                let f = measure_lit(library, model, &["upperarm", "forearm"])?;
                notes.push(format!("sleeve colour measured on {model}: {:.0}, {:.0}, {:.0}", f[0], f[1], f[2]));
                f
            }
            Skin::Unchanged => return Err("the sleeve needs a fabric colour".into()),
        };
        let band = s.band.map_or(fabric, |(c, _)| c.map(|v| v as f32));
        let width = s.band.map_or(0.0, |(_, w)| w);
        sleeve_colours = Some((fabric, band));
        let mut cuffs = vec![s.cuff];
        cuffs.extend(r.per_mesh_cuff.iter().map(|(_, c)| *c));
        cuffs.sort_by(f32::total_cmp);
        cuffs.dedup();
        for (path, w, h, along, around, n) in &placed {
            for &cuff in &cuffs {
                let mask = cloth_mask_with(along, around, *w, *h, cuff, width, s.band_at, s.creases);
                notes.push(format!("sleeve on {path} (cuff {cuff}): {} texels of cloth, from {n} mesh(es)", mask.iter().filter(|c| **c != Cloth::None).count()));
                cloth.insert(cloth_key(path, cuff), (mask, fabric, band));
            }
        }
    }
    // The gloves: their colour and the texels they cover.
    let mut gloved: HashMap<String, (Vec<bool>, [f32; 3])> = HashMap::new();
    if let Some(g) = &r.gloves {
        let colour = match &g.colour {
            Skin::Color(c) => c.map(|v| v as f32),
            Skin::From(model) => {
                let c = measure(library, model, &["hand", "finger"])?;
                notes.push(format!("gloves' colour measured on {model}: {:.0}, {:.0}, {:.0}", c[0], c[1], c[2]));
                c
            }
            Skin::Unchanged => return Err("the gloves need a colour".into()),
        };
        for (path, w, h, along, _, _) in &placed {
            let mut mask: Vec<bool> = along.iter().map(|t| t.is_some_and(|t| t >= g.from && g.to.map_or(true, |to| t < to))).collect();
            // Into the texels of no face around them too (as the sleeve's cloth), so that the
            // texture's filtering does not bring the skin back on the gloves' edges.
            let (wi, hi) = (*w as i64, *h as i64);
            for _ in 0..12 {
                let prev = mask.clone();
                for y in 0..hi {
                    for x in 0..wi {
                        let i = (y * wi + x) as usize;
                        if prev[i] || along[i].is_some() {
                            continue;
                        }
                        mask[i] = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| {
                            let (nx, ny) = (x + dx, y + dy);
                            nx >= 0 && ny >= 0 && nx < wi && ny < hi && prev[(ny * wi + nx) as usize]
                        });
                    }
                }
            }
            notes.push(format!("gloves on {path}: {} texels", mask.iter().filter(|m| **m).count()));
            gloved.insert(path.clone(), (mask, colour));
        }
    }
    // A sleeve in relief: where the model's sleeve lies in its texture.
    let mut model_cloth: Option<(u32, u32, Vec<u8>)> = None;
    let shell = match r.sleeve.as_ref().and_then(|s| s.model.as_ref().map(|m| (s.cuff, m))) {
        Some((cuff, m)) if m.painted => {
            notes.push(format!("sleeve in relief drawn plain, {} fold(s)", m.folds));
            Some((cuff, m.clone(), None))
        }
        Some((cuff, m)) => {
            // The model's printed cloth, its lit half only, in the sleeve's own texture.
            let uv = sleeve_uv(library, &m.from)?;
            notes.push(format!("sleeve in relief with the cloth of {} ({}, lit half, mirrored around)", m.from, uv[0].0));
            model_cloth = Some(model_lit_texture(library, &uv[0])?);
            Some((cuff, m.clone(), None::<[SleeveUv; 2]>))
        }
        None => None,
    };
    let recolour_on = target.is_some() || r.nails.is_some() || r.sleeve.is_some() || r.gloves.is_some();
    let mut recoloured: HashMap<String, String> = HashMap::new();
    let mut new_textures = Vec::new();
    // The plain sleeve's own texture.
    let sleeve_name = format!("{name}.sleeve");
    if let (Some((_, m, None)), Some(s), Some((fabric, band))) = (&shell, &r.sleeve, sleeve_colours) {
        let band = s.band.map(|(_, wd)| {
            let lo = s.band_at.unwrap_or(s.cuff - wd);
            (band, lo, lo + wd)
        });
        let (w, h, rgba) = match model_cloth.take() {
            Some(cloth) => cloth,
            None => sleeve_texture(fabric, band, s, m.seam),
        };
        new_textures.push((sleeve_name.clone(), w, h, rgba));
    }
    let mut meshes = Vec::new();
    for (n, from) in list.into_iter().enumerate() {
        let (mut mesh, bones, textures, animation, aliases, cartoon) = sh.meshes[n].clone();
        if !bones.iter().any(|b| kind(b) != Kind::Other) {
            return Err(format!("{from} has no first-person arm bones (FPS_…)"));
        }
        let shape = r.per_mesh.iter().find(|(m, _)| m.eq_ignore_ascii_case(&from)).map_or(r.shape, |(_, s)| *s);
        let mesh_cuff = r.sleeve.as_ref().map(|s| r.per_mesh_cuff.iter().find(|(m, _)| m.eq_ignore_ascii_case(&from)).map_or(s.cuff, |(_, c)| *c));
        thin(&mut mesh, &bones, &shape);
        // A hidden arm: its faces left out (a face with one point on that arm).
        if let Some(side) = r.hide_arm {
            let on = |p: usize| main_bone(&mesh.influences[p]).is_some_and(|b| kind(&bones[b]) != Kind::Other && bones[b].to_ascii_lowercase().starts_with("fpsg") == (side == Side::Left));
            let keep: Vec<bool> = mesh.faces.iter().map(|f| !f.wedges.iter().any(|&w| on(mesh.wedges[w as usize].point as usize))).collect();
            let mut k = keep.iter();
            mesh.faces.retain(|_| *k.next().unwrap());
        }
        // Faces' material = texture index (the parts of a mod's model).
        let tex_of = face_textures(&mesh);
        for (f, t) in mesh.faces.iter_mut().zip(&tex_of) {
            f.material = *t as u16;
        }
        mesh.materials = (0..textures.len()).map(|t| (0, t as i32)).collect();
        let mut part_textures = textures.clone();
        if let Some(i) = arms_texture(&mesh, &bones, textures.len()).filter(|_| recolour_on) {
            let path = textures[i].to_ascii_lowercase();
            let key = mesh_cuff.map_or(path.clone(), |c| cloth_key(&path, c));
            if !recoloured.contains_key(&key) {
                let (mut w, mut h, mut rgba) = shared_texture(sh, library, &textures[i])?;
                if cloth.contains_key(&key) && SLEEVE_SCALE > 1 {
                    (w, h, rgba) = upscale(w, h, &rgba, SLEEVE_SCALE);
                }
                let sleeve = cloth.get(&key).map(|(m, f, b)| (m.as_slice(), *f, *b));
                let gloves = gloved.get(&path).map(|(m, c)| (m.as_slice(), *c));
                let (nails, base) = recolour_with(w, h, &mut rgba, target, r.nails, sleeve, gloves);
                notes.push(format!("texture {} recoloured (its skin: {:.0}, {:.0}, {:.0}; {nails} nail texels)", textures[i], base[0], base[1], base[2]));
                let texture = if recoloured.is_empty() { format!("{name}.skin") } else { format!("{name}.skin{}", recoloured.len() + 1) };
                recoloured.insert(key.clone(), texture.clone());
                new_textures.push((texture, w, h, rgba));
            }
            let new = recoloured[&key].clone();
            for t in part_textures.iter_mut().filter(|t| t.eq_ignore_ascii_case(&path)) {
                *t = new.clone();
            }
        }
        if let Some((cuff, m, uv)) = &shell {
            let mut part = |t: &str, parts: &mut Vec<String>| match parts.iter().position(|p| p.eq_ignore_ascii_case(t)) {
                Some(i) => i as u16,
                None => {
                    parts.push(t.to_string());
                    (parts.len() - 1) as u16
                }
            };
            let parts = match uv {
                Some(uv) => Some([part(&uv[0].0, &mut part_textures), part(&uv[1].0, &mut part_textures)]),
                None => {
                    let p = part(&sleeve_name, &mut part_textures);
                    Some([p, p])
                }
            };
            if let Some(parts) = parts {
                add_sleeve_shell(&mut mesh, &bones, mesh_cuff.unwrap_or(*cuff), m, uv.as_ref(), parts, r.hide_arm);
            }
        }
        // Bones named by their index (as a mod's model).
        for (i, b) in mesh.bones.iter_mut().enumerate() {
            b.name = i as i32;
        }
        let bounds = mesh.points.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), p| ([0, 1, 2].map(|k| lo[k].min(p[k])), [0, 1, 2].map(|k| hi[k].max(p[k]))));
        let bone_boxes = crate::mod_skeletal::bone_boxes(&mesh);
        meshes.push((from, ModSkeletal { mesh, bone_names: bones, textures: part_textures, animation_name: animation, aliases, bounds, bone_boxes, cartoon, game_look: true }));
    }
    if meshes.is_empty() {
        return Err("no first-person mesh found".into());
    }
    Ok(Built { meshes, textures: new_textures, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bone_kinds() {
        assert_eq!(kind("FPS_index02"), Kind::Finger);
        assert_eq!(kind("FPSg_pouce01"), Kind::Finger);
        assert_eq!(kind("FPS_main01"), Kind::Hand);
        assert_eq!(kind("FPSg_bras"), Kind::Forearm);
        assert_eq!(kind("berretta"), Kind::Other);
        assert_eq!(kind("chargeur"), Kind::Other);
        assert_eq!(kind("Box01"), Kind::Other);
    }

    /// A tube along X held by two bones (forearm then hand): thinner around the axis, not
    /// shorter; a point on the weapon's bone does not move.
    #[test]
    fn thinning_keeps_the_axis_and_the_weapon() {
        use crate::skeletal::{Bone, Wedge};
        let bone = |name: i32, parent: i32, x: f32| Bone { name, flags: 0, orientation: [0.0, 0.0, 0.0, 1.0], position: [x, 0.0, 0.0], parent };
        let mut mesh = SkeletalMesh {
            version: 1,
            legacy_vertices: Vec::new(),
            legacy_triangles: Vec::new(),
            textures: Vec::new(),
            scale: [1.0; 3],
            origin: [0.0; 3],
            rot_origin: [0; 3],
            faces: Vec::new(),
            wedges: vec![Wedge { point: 0, uv: [0.0; 2] }],
            materials: Vec::new(),
            points: vec![[5.0, 2.0, 0.0], [12.0, 0.0, 3.0], [12.0, 0.0, 3.0]],
            // Root forearm at 0, hand at 10 (child), weapon at 10 (child of the hand).
            bones: vec![bone(0, 0, 0.0), bone(1, 0, 10.0), bone(2, 1, 0.0)],
            animation: 0,
            influences: vec![vec![(0, 1.0)], vec![(1, 1.0)], vec![(2, 1.0)]],
        };
        let names = vec!["FPS_bras".to_string(), "FPS_main01".to_string(), "berretta".to_string()];
        let r = Shape { fingers: 0.8, hand: 0.5, wrist: 0.6, forearm: 1.0 };
        thin(&mut mesh, &names, &r);
        // Forearm half way: factor 0.8, along X unchanged.
        assert!((mesh.points[0][0] - 5.0).abs() < 1e-5 && (mesh.points[0][1] - 1.6).abs() < 1e-5);
        // Hand: 3 → 1.5 around its axis (the direction from its parent).
        assert!((mesh.points[1][0] - 12.0).abs() < 1e-5 && (mesh.points[1][2] - 1.5).abs() < 1e-5);
        // Weapon: unchanged.
        assert_eq!(mesh.points[2], [12.0, 0.0, 3.0]);
    }

    #[test]
    fn recolour_keeps_shading_and_finds_nails() {
        // 8×8 skin with two shades (a darker column), and a 3×3 nail block in a corner.
        let (w, h) = (8u32, 8u32);
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let c = if x < 3 && y < 3 { [250, 188, 177] } else if x == 7 { [200, 140, 110] } else { [230, 160, 130] };
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let (nails, base) = recolour(w, h, &mut rgba, Some([115.0, 80.0, 65.0]), Some([200, 150, 140]));
        assert!(nails >= 1);
        assert_eq!(base, [230.0, 160.0, 130.0]);
        // A skin texel with the median colour becomes the target.
        let px = |x: u32, y: u32| &rgba[((y * w + x) * 4) as usize..((y * w + x) * 4 + 3) as usize];
        assert_eq!(px(0, 7), &[115, 80, 65]);
        // The darker shade stays darker.
        assert!(px(7, 7)[0] < 115);
        // The middle of the nail block is a nail: the nail colour at its own shading.
        assert_eq!(px(1, 1), &[200, 150, 140]);
    }

    /// The arms of the 9mm from the player's game (OXIII_GAME; skipped when it is not set): same
    /// bones, points and faces count, thinner fingers, untouched weapon.
    #[test]
    fn arms_of_the_9mm() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| openxiii_package::system_dir(g).exists()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let mut library = Library::new(&game).unwrap();
        let r = Recipe {
            from: vec!["XIIIArmes.FpsBerrettaM".into()],
            skip: Vec::new(),
            shape: Shape { fingers: 0.8, hand: 0.85, wrist: 0.8, forearm: 0.9 },
            per_mesh: Vec::new(),
            per_mesh_cuff: Vec::new(),
            skin: Skin::From("XIIIPersos.JonesM".into()),
            nails: Some([210, 160, 150]),
            sleeve: None,
            gloves: None,
            hide_arm: None,
        };
        let started = std::time::Instant::now();
        let built = build(&mut library, "Mod.Test", &r).expect("arms");
        eprintln!("made in {:.1} ms; {:?}", started.elapsed().as_secs_f64() * 1000.0, built.notes);
        let (game_mesh, bones, ..) = read_mesh(&mut library, "XIIIArmes.FpsBerrettaM").unwrap();
        let m = &built.meshes[0].1.mesh;
        assert_eq!(m.points.len(), game_mesh.points.len());
        assert_eq!(m.faces.len(), game_mesh.faces.len());
        assert_eq!(built.textures.len(), 1);
        assert!(built.meshes[0].1.textures.iter().any(|t| t == "Mod.Test.skin"));
        let mut moved = 0;
        for (p, (a, b)) in m.points.iter().zip(&game_mesh.points).enumerate() {
            let only_weapon = m.influences[p].iter().all(|&(bone, _)| kind(&bones[bone as usize]) == Kind::Other);
            if only_weapon {
                assert_eq!(a, b, "a point of the weapon moved");
            } else if a != b {
                moved += 1;
            }
        }
        assert!(moved > game_mesh.points.len() / 3, "only {moved} points moved");
    }

    /// The cloth of a texel follows its place along the forearm; a texel of no face next to the
    /// sleeve takes its cloth, not one next to the skin.
    #[test]
    fn cloth_follows_the_forearm() {
        // 4 × 1: elbow, middle, band, hand; then 4 × 1 more with an empty texel after the elbow.
        let along = [Some(0.0), Some(0.5), Some(0.88), Some(1.5), None, Some(0.2), None, Some(1.5)];
        let m = cloth_mask(&along, 4, 2, 0.9, 0.05);
        assert_eq!(&m[..4], &[Cloth::Fabric, Cloth::Fabric, Cloth::Band, Cloth::None]);
        assert_eq!(m[4], Cloth::Fabric);
        assert_eq!(m[5], Cloth::Fabric);
        assert_eq!(m[7], Cloth::None);
    }

    /// Carrington's sleeves on the game's 9mm (OXIII_GAME; skipped when it is not set): the
    /// forearm's texels are cloth, the hand's are not, the fabric's colour is measured.
    #[test]
    fn carrington_sleeves_on_the_9mm() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| openxiii_package::system_dir(g).exists()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let mut library = Library::new(&game).unwrap();
        let (mesh, bones, ..) = read_mesh(&mut library, "XIIIArmes.FpsBerrettaM").unwrap();
        let tex = arms_texture(&mesh, &bones, mesh.textures.len()).unwrap();
        let along = along_forearm(&[(&mesh, bones.as_slice(), tex)], 512, 512);
        let mask = cloth_mask(&along, 512, 512, 0.97, 0.0);
        let cloth = mask.iter().filter(|c| **c != Cloth::None).count();
        let hands = along.iter().filter(|t| t.is_some_and(|t| t > 1.2)).count();
        assert!(cloth > 20_000 && hands > 20_000, "cloth {cloth}, hands {hands}");
        // No texel of the hand (past the wrist) is cloth.
        assert!(along.iter().zip(&mask).all(|(t, c)| !(t.is_some_and(|t| t > 1.0) && *c != Cloth::None)));
        let r = Recipe {
            from: vec!["XIIIArmes.FpsBerrettaM".into()],
            skip: Vec::new(),
            shape: Shape { fingers: 1.0, hand: 1.0, wrist: 1.04, forearm: 1.08 },
            per_mesh: Vec::new(),
            per_mesh_cuff: Vec::new(),
            skin: Skin::From("XIIIPersos.CarringtonM".into()),
            nails: None,
            sleeve: Some(Sleeve { fabric: Skin::From("XIIIPersos.CarringtonM".into()), cuff: 0.97, band: None, band_at: None, creases: false, rings: Vec::new(), ring_colour: None, tab: None, ribs: None, checks: None, buttons: None, model: Some(SleeveModel { from: "XIIIPersos.CarringtonM".into(), thickness: 1.1, room: 0.3, lip: 1.08, seam: 0.0, painted: false, folds: 0, fold_depth: 0.0, fold_width: 0.1, fold_slant: 0.0, taper: None }) }),
            gloves: None,
            hide_arm: None,
        };
        let built = build(&mut library, "Mod.C", &r).unwrap();
        assert!(built.notes.iter().any(|n| n.starts_with("sleeve colour measured")), "{:?}", built.notes);
    }

    /// Every first-person mesh of the game's two packages (OXIII_GAME; skipped when it is not
    /// set): one skin texture shared, no point of an object moved, every mesh thinner, a
    /// skipped mesh and a per-mesh shape honoured.
    #[test]
    fn all_first_person_meshes() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| openxiii_package::system_dir(g).exists()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let mut library = Library::new(&game).unwrap();
        let unchanged = Shape { fingers: 1.0, hand: 1.0, wrist: 1.0, forearm: 1.0 };
        let r = Recipe {
            from: vec!["XIIIArmes.*".into(), "XIIIDeco.*".into()],
            skip: vec!["XIIIArmes.fpsM".into()],
            shape: Shape { fingers: 0.75, hand: 0.8, wrist: 0.7, forearm: 0.85 },
            per_mesh: vec![("XIIIArmes.fpsuziM".into(), unchanged)],
            per_mesh_cuff: Vec::new(),
            skin: Skin::Color([165, 97, 41]),
            nails: None,
            sleeve: None,
            gloves: None,
            hide_arm: None,
        };
        let started = std::time::Instant::now();
        let built = build(&mut library, "Mod.All", &r).expect("arms");
        eprintln!("{} meshes made in {:.1} ms; {:?}", built.meshes.len(), started.elapsed().as_secs_f64() * 1000.0, built.notes);
        // 38 in XIIIArmes, 11 in XIIIDeco, one skipped.
        assert_eq!(built.meshes.len(), 48);
        assert_eq!(built.textures.len(), 1);
        assert!(!built.meshes.iter().any(|(m, _)| m.eq_ignore_ascii_case("XIIIArmes.fpsM")));
        for (from, made) in &built.meshes {
            let (game_mesh, bones, ..) = read_mesh(&mut library, from).unwrap();
            let mut moved = 0;
            for (p, (a, b)) in made.mesh.points.iter().zip(&game_mesh.points).enumerate() {
                if made.mesh.influences[p].iter().all(|&(bone, _)| kind(&bones[bone as usize]) == Kind::Other) {
                    assert_eq!(a, b, "{from}: a point of the object moved");
                } else if a != b {
                    moved += 1;
                }
            }
            if from.eq_ignore_ascii_case("XIIIArmes.fpsuziM") {
                assert_eq!(moved, 0, "per-mesh shape not applied");
            } else {
                assert!(moved > 0, "{from}: nothing thinner");
            }
            assert!(made.textures.iter().any(|t| t == "Mod.All.skin"), "{from}: skin not recoloured");
        }
    }

}

