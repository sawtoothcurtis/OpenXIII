//! A mod's own 3D models: `[models.<Name>]` tables, read from glTF 2.0 files of the mod folder.
//!
//! ```toml
//! [models.Crate]                # a new static mesh, named "Mod.Crate" wherever a mesh is accepted
//! file = "models/crate.gltf"    # .gltf (with its .bin and image files) or .glb
//! scale = 50                    # game units per glTF metre (optional, 50 by default)
//! replace = "Package.Group.Mesh"  # also drawn instead of this game static mesh (optional)
//! collision = "mesh"            # what blocks: "mesh" (its triangles, default), "box", "none"
//! outline = true                # black comic-book outline, as on the game's objects (default true)
//! cartoon = true                # flat comic-book shading, as on the game's objects (default true)
//! outline_width = 1.0           # outline thickness, 1.0 = the game's (2.0: twice as thick)
//!
//! [models.Guard.animations]     # animated models only: game animation = glTF animation
//! Fire = "shoot"
//! WalkF = { clip = "walk", notifies = [[0.25, "PlayFootStep"], [0.75, "PlayFootStep"]] }
//! ```
//!
//! glTF is right-handed, Y up, in metres, facing -Z; the game is Z up, facing +X. A glTF point
//! (x, y, z) becomes (-z, x, y) × scale. Each primitive (triangles) becomes one section; its
//! base colour texture, when it is a PNG file next to the model, becomes the section's texture.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use toml::Spanned;

use crate::placement::Source;
use crate::{line_of, Problem};

/// Prefix of the names of the mods' own models.
pub const PREFIX: &str = "Mod.";
/// Default `scale`: game units per glTF metre.
pub const DEFAULT_SCALE: f32 = 50.0;

/// What the actors drawn with a model collide with (`collision` key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Collision {
    /// The model's own triangles (default).
    #[default]
    Mesh,
    /// The box around the model's vertices: closed and cheap.
    Box,
    /// Nothing: players, characters and shots go through it.
    None,
}

/// A `[models.<Name>]` definition.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnModel {
    pub name: String,
    pub file: PathBuf,
    pub scale: f32,
    pub replace: Option<String>,
    /// Game material (texture) every part is drawn with, if given.
    pub material: Option<String>,
    pub collision: Collision,
    /// Black outline (UPrimitive+0x28), on by default as on the game's meshes.
    pub outline: bool,
    /// Flat cartoon shading (UPrimitive+0x29), on by default as on the game's meshes.
    pub cartoon: bool,
    /// Outline thickness factor (UPrimitive+0x2C): 1.0, the game's own, by default.
    pub outline_width: f32,
    /// Animated models: the game animations made from the glTF animations.
    pub animations: Vec<AnimationLink>,
    /// Animated models: bone names the game's scripts use → the model's bones
    /// (`[models.<Name>.bones]`).
    pub bones: Vec<(String, String)>,
    pub source: Source,
}

/// One game animation made from a glTF animation (`[models.<Name>.animations]`).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationLink {
    /// The name the game's scripts play ("Fire", "WalkF"…).
    pub game: String,
    /// The glTF animation it is made from.
    pub clip: String,
    /// (time as a fraction of the animation, 0 to 1, script function called).
    pub notifies: Vec<(f32, String)>,
}

impl OwnModel {
    /// Full mesh name: "Mod.<Name>".
    pub fn mesh_name(&self) -> String {
        format!("{PREFIX}{}", self.name)
    }
}

/// One section of a model, in game coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPart {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// PNG file of the base colour texture, if any.
    pub texture: Option<PathBuf>,
}

pub(crate) type ModelTables = BTreeMap<String, Spanned<toml::Table>>;

/// Reads the `[models.<Name>]` tables of a file of the mod in folder `mod_dir`.
pub(crate) fn read(text: &str, file: &Path, mod_name: &str, mod_dir: &Path, tables: ModelTables, out: &mut Vec<OwnModel>, problems: &mut Vec<Problem>) {
    for (name, t) in tables {
        let line = line_of(text, t.span().start);
        let err = |m: String| Problem::Error { file: file.to_path_buf(), line: Some(line), message: format!("models.{name}: {m}; model skipped") };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            problems.push(err("the name may only use letters, digits, _ and -".into()));
            continue;
        }
        let t = t.get_ref();
        if let Some(k) = t.keys().find(|k| !["file", "scale", "replace", "material", "collision", "outline", "cartoon", "outline_width", "animations", "bones"].contains(&k.as_str())) {
            problems.push(err(format!("unknown key {k} (keys: file, scale, replace, material, collision, outline, cartoon, outline_width, animations, bones)")));
            continue;
        }
        let Some(toml::Value::String(f)) = t.get("file") else {
            problems.push(err("file = \"path inside the mod folder\" is missing".into()));
            continue;
        };
        let rel = Path::new(f);
        if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            problems.push(err(format!("file {f:?} must be inside the mod folder")));
            continue;
        }
        let scale = match t.get("scale") {
            None => DEFAULT_SCALE,
            Some(toml::Value::Integer(i)) if *i > 0 => *i as f32,
            Some(toml::Value::Float(x)) if *x > 0.0 => *x as f32,
            Some(v) => {
                problems.push(err(format!("scale = {v} must be a positive number")));
                continue;
            }
        };
        let replace = match t.get("replace") {
            None => None,
            Some(toml::Value::String(r)) if r.contains('.') => Some(r.trim().to_string()),
            Some(v) => {
                problems.push(err(format!("replace = {v} must be a game static mesh \"Package.Group.Name\"")));
                continue;
            }
        };
        let material = match t.get("material") {
            None => None,
            Some(toml::Value::String(m)) if m.contains('.') => Some(m.trim().to_string()),
            Some(v) => {
                problems.push(err(format!("material = {v} must be a game texture \"Package.Group.Name\"")));
                continue;
            }
        };
        let collision = match t.get("collision") {
            None => Collision::Mesh,
            Some(toml::Value::String(c)) if c.eq_ignore_ascii_case("mesh") => Collision::Mesh,
            Some(toml::Value::String(c)) if c.eq_ignore_ascii_case("box") => Collision::Box,
            Some(toml::Value::String(c)) if c.eq_ignore_ascii_case("none") => Collision::None,
            Some(v) => {
                problems.push(err(format!("collision = {v} must be \"mesh\", \"box\" or \"none\"")));
                continue;
            }
        };
        let flag = |key: &str| match t.get(key) {
            None => Ok(true),
            Some(toml::Value::Boolean(b)) => Ok(*b),
            Some(v) => Err(format!("{key} = {v} must be true or false")),
        };
        let (outline, cartoon) = match (flag("outline"), flag("cartoon")) {
            (Ok(o), Ok(c)) => (o, c),
            (Err(e), _) | (_, Err(e)) => {
                problems.push(err(e));
                continue;
            }
        };
        let outline_width = match t.get("outline_width") {
            None => 1.0,
            Some(toml::Value::Integer(i)) if *i > 0 && *i <= 20 => *i as f32,
            Some(toml::Value::Float(x)) if *x > 0.0 && *x <= 20.0 => *x as f32,
            Some(v) => {
                problems.push(err(format!("outline_width = {v} must be a number above 0 and up to 20 (1.0 = the game's thickness)")));
                continue;
            }
        };
        let animations = match t.get("animations") {
            None => Vec::new(),
            Some(toml::Value::Table(a)) => match read_animations(a) {
                Ok(list) => list,
                Err(e) => {
                    problems.push(err(format!("animations: {e}")));
                    continue;
                }
            },
            Some(v) => {
                problems.push(err(format!("animations = {v} must be a table, for example {{ Fire = \"shoot\" }}")));
                continue;
            }
        };
        let bones = match t.get("bones") {
            None => Vec::new(),
            Some(toml::Value::Table(b)) if b.values().all(|v| v.is_str()) => b.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect(),
            Some(v) => {
                problems.push(err(format!("bones = {v} must be a table of names, for example {{ \"X R Hand\" = \"hand_r\" }}")));
                continue;
            }
        };
        out.push(OwnModel { name, file: crate::file_in_mod(mod_dir, f), scale, replace, material, collision, outline, cartoon, outline_width, animations, bones, source: Source { mod_name: mod_name.to_string(), file: file.to_path_buf(), line } });
    }
}

/// The links of an `animations` table: `Game = "clip"` or `Game = { clip = "…", notifies = [[t, "Function"], …] }`.
fn read_animations(t: &toml::Table) -> Result<Vec<AnimationLink>, String> {
    let mut out = Vec::new();
    for (game, v) in t {
        if game.is_empty() || !game.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("\"{game}\" is not an animation name (letters, digits and _)"));
        }
        let (clip, notifies) = match v {
            toml::Value::String(c) => (c.clone(), Vec::new()),
            toml::Value::Table(x) => {
                if let Some(k) = x.keys().find(|k| !["clip", "notifies"].contains(&k.as_str())) {
                    return Err(format!("{game}: unknown key {k} (keys: clip, notifies)"));
                }
                let Some(toml::Value::String(c)) = x.get("clip") else { return Err(format!("{game}: clip = \"glTF animation name\" is missing")) };
                let mut notifies = Vec::new();
                match x.get("notifies") {
                    None => {}
                    Some(toml::Value::Array(items)) => {
                        for item in items {
                            let pair = match item {
                                toml::Value::Array(p) if p.len() == 2 => p,
                                _ => return Err(format!("{game}: each notify is [time, \"Function\"]")),
                            };
                            let time = match &pair[0] {
                                toml::Value::Float(f) => *f as f32,
                                toml::Value::Integer(i) => *i as f32,
                                _ => return Err(format!("{game}: a notify's time is a number from 0 to 1")),
                            };
                            let toml::Value::String(f) = &pair[1] else { return Err(format!("{game}: a notify's function is a string")) };
                            if !(0.0..=1.0).contains(&time) {
                                return Err(format!("{game}: notify time {time} must be from 0 to 1 (a fraction of the animation)"));
                            }
                            notifies.push((time, f.clone()));
                        }
                    }
                    Some(_) => return Err(format!("{game}: notifies = [[time, \"Function\"], …]")),
                }
                (c.clone(), notifies)
            }
            _ => return Err(format!("{game} = {v}: write \"glTF animation name\" or {{ clip = \"…\" }}")),
        };
        out.push(AnimationLink { game: game.clone(), clip, notifies });
    }
    Ok(out)
}

type Mat4 = [[f32; 4]; 4];

fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    // Column-major: r[c][row] = sum a[k][row] * b[c][k].
    let mut r = [[0.0; 4]; 4];
    for c in 0..4 {
        for row in 0..4 {
            r[c][row] = (0..4).map(|k| a[k][row] * b[c][k]).sum();
        }
    }
    r
}

fn point(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    let v = |row: usize| m[0][row] * p[0] + m[1][row] * p[1] + m[2][row] * p[2] + m[3][row];
    [v(0), v(1), v(2)]
}

fn direction(m: &Mat4, d: [f32; 3]) -> [f32; 3] {
    let v = |row: usize| m[0][row] * d[0] + m[1][row] * d[1] + m[2][row] * d[2];
    let r = [v(0), v(1), v(2)];
    let l = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
    if l > 0.0 {
        [r[0] / l, r[1] / l, r[2] / l]
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// glTF point to game point.
fn to_game(p: [f32; 3], scale: f32) -> [f32; 3] {
    [-p[2] * scale, p[0] * scale, p[1] * scale]
}

/// Vertices of a whole model (all its parts, a mesh placed by several nodes counted each time).
pub const MAX_MODEL_VERTICES: usize = 1 << 20;

/// Reads a glTF 2.0 model: its triangles in game coordinates, one part per primitive.
pub fn load_gltf(path: &Path, scale: f32) -> Result<Vec<ModelPart>, String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let bytes = std::fs::read(path).map_err(|e| at(&e))?;
    let g = gltf::Gltf::from_slice(&bytes).map_err(|e| at(&e))?;
    let dir = path.parent().unwrap_or(Path::new(""));
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for b in g.document.buffers() {
        let data = match b.source() {
            gltf::buffer::Source::Bin => g.blob.clone().ok_or_else(|| at(&"the .glb has no binary chunk"))?,
            gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => return Err(at(&"embedded data URIs are not supported: use a .glb or separate .bin files")),
            gltf::buffer::Source::Uri(uri) => {
                let file = crate::path_in(dir, uri).ok_or_else(|| at(&format!("{uri}: the file must be inside the mod folder")))?;
                std::fs::read(file).map_err(|e| at(&format!("{uri}: {e}")))?
            }
        };
        buffers.push(data);
    }
    crate::check_gltf_data(&g.document, &buffers).map_err(|e| at(&e))?;
    crate::check_gltf_tree(&g.document).map_err(|e| at(&e))?;
    let scene = g.document.default_scene().or_else(|| g.document.scenes().next()).ok_or_else(|| at(&"no scene"))?;
    let mut parts = Vec::new();
    let mut total_vertices = 0usize;
    let identity: Mat4 = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let mut todo: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, identity)).collect();
    while let Some((node, parent)) = todo.pop() {
        let m = mul(&parent, &node.transform().matrix());
        for child in node.children() {
            todo.push((child, m));
        }
        let Some(mesh) = node.mesh() else { continue };
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = prim.reader(|b| buffers.get(b.index()).map(|v| v.as_slice()));
            let Some(positions) = reader.read_positions() else { continue };
            let positions: Vec<[f32; 3]> = positions.map(|p| to_game(point(&m, p), scale)).collect();
            if positions.len() > 65535 {
                return Err(at(&format!("a part has {} vertices, at most 65535 are supported", positions.len())));
            }
            total_vertices += positions.len();
            if total_vertices > MAX_MODEL_VERTICES {
                return Err(at(&format!("the model has more than {MAX_MODEL_VERTICES} vertices in all")));
            }
            let normals: Vec<[f32; 3]> = match reader.read_normals() {
                Some(n) => n.map(|d| to_game(direction(&m, d), 1.0)).collect(),
                None => vec![[0.0, 0.0, 1.0]; positions.len()],
            };
            let uvs: Vec<[f32; 2]> = match reader.read_tex_coords(0) {
                Some(t) => t.into_f32().collect(),
                None => vec![[0.0, 0.0]; positions.len()],
            };
            let indices: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            if indices.iter().any(|&i| i as usize >= positions.len()) {
                return Err(at(&"an index points past the vertices"));
            }
            if normals.len() != positions.len() || uvs.len() != positions.len() {
                return Err(at(&"normals or texture coordinates do not match the vertices"));
            }
            let texture = prim.material().pbr_metallic_roughness().base_color_texture().and_then(|t| match t.texture().source().source() {
                gltf::image::Source::Uri { uri, .. } if uri.to_ascii_lowercase().ends_with(".png") && !uri.starts_with("data:") => crate::path_in(dir, uri),
                _ => None,
            });
            parts.push(ModelPart { positions, normals, uvs, indices, texture });
        }
    }
    if parts.is_empty() {
        return Err(at(&"no triangles in the default scene"));
    }
    Ok(parts)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::defaults::parse_mod_file;

    /// A one-triangle glTF model with a separate .bin and a texture reference.
    pub(crate) fn write_triangle(dir: &Path) -> PathBuf {
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let indices: [u16; 3] = [0, 1, 2];
        let mut bin = Vec::new();
        positions.iter().for_each(|v| bin.extend(v.to_le_bytes()));
        indices.iter().for_each(|v| bin.extend(v.to_le_bytes()));
        bin.extend([0u8; 2]);
        std::fs::write(dir.join("tri.bin"), &bin).unwrap();
        let json = r#"{
  "asset": {"version": "2.0"},
  "scene": 0,
  "scenes": [{"nodes": [0]}],
  "nodes": [{"mesh": 0, "translation": [0, 0, -2]}],
  "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
  "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
  "textures": [{"source": 0}],
  "images": [{"uri": "tri.png"}],
  "buffers": [{"uri": "tri.bin", "byteLength": 44}],
  "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 6}],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]},
    {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
  ]
}"#;
        let path = dir.join("tri.gltf");
        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn tables_and_errors() {
        let text = "[models.Crate]\nfile = \"models/crate.glb\"\n\n[models.Big]\nfile = \"big.gltf\"\nscale = 100\nreplace = \"XIIIPlage.Rock01\"\n\n[models.Bad]\nfile = \"x.glb\"\nscale = -1\n\n[models.Solid]\nfile = \"s.glb\"\ncollision = \"box\"\n\n[models.Ghost]\nfile = \"g.glb\"\ncollision = \"NONE\"\n\n[models.Odd]\nfile = \"o.glb\"\ncollision = \"sphere\"\n";
        let f = parse_mod_file(text, Path::new("m/mod.toml"), "m");
        assert_eq!(f.models.len(), 4, "{:?}", f.models);
        let kind = |n: &str| f.models.iter().find(|m| m.name == n).map(|m| m.collision);
        assert_eq!((kind("Crate"), kind("Solid"), kind("Ghost"), kind("Odd")), (Some(Collision::Mesh), Some(Collision::Box), Some(Collision::None), None));
        assert!(f.problems.iter().any(|p| p.to_string().contains("(line 21)") && p.to_string().contains("\"box\"")), "{:?}", f.problems);
        let c = f.models.iter().find(|m| m.name == "Crate").unwrap();
        assert_eq!((c.mesh_name().as_str(), c.scale), ("Mod.Crate", DEFAULT_SCALE));
        let b = f.models.iter().find(|m| m.name == "Big").unwrap();
        assert_eq!((b.scale, b.replace.as_deref()), (100.0, Some("XIIIPlage.Rock01")));
        assert!(f.problems.iter().any(|p| p.to_string().contains("(line 9)") && p.to_string().contains("positive")), "{:?}", f.problems);
    }

    #[test]
    fn outline_and_cartoon_keys() {
        let text = "[models.Crate]
file = \"c.glb\"

[models.Flat]
file = \"f.glb\"
outline = false
cartoon = false

[models.Bad]
file = \"b.glb\"
outline = \"no\"
";
        let f = parse_mod_file(text, Path::new("m/mod.toml"), "m");
        let flags = |n: &str| f.models.iter().find(|m| m.name == n).map(|m| (m.outline, m.cartoon));
        assert_eq!((flags("Crate"), flags("Flat"), flags("Bad")), (Some((true, true)), Some((false, false)), None));
        assert!(f.problems.iter().any(|p| p.to_string().contains("true or false")), "{:?}", f.problems);
        let text = "[models.A]\nfile = \"a.glb\"\n\n[models.B]\nfile = \"b.glb\"\noutline_width = 2\n\n[models.C]\nfile = \"c.glb\"\noutline_width = 0.5\n\n[models.D]\nfile = \"d.glb\"\noutline_width = 0\n";
        let f = parse_mod_file(text, Path::new("m/mod.toml"), "m");
        let width = |n: &str| f.models.iter().find(|m| m.name == n).map(|m| m.outline_width);
        assert_eq!((width("A"), width("B"), width("C"), width("D")), (Some(1.0), Some(2.0), Some(0.5), None));
        assert!(f.problems.iter().any(|p| p.to_string().contains("outline_width")), "{:?}", f.problems);
    }

    #[test]
    fn triangle_in_game_coordinates() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_triangle(tmp.path());
        let parts = load_gltf(&path, 50.0).unwrap();
        assert_eq!(parts.len(), 1);
        let p = &parts[0];
        // glTF (0,0,-2) (node translation) is 2 m ahead: game (+100, 0, 0).
        assert_eq!(p.positions[0], [100.0, 0.0, 0.0]);
        // glTF +X is game +Y, glTF +Y is game +Z.
        assert_eq!(p.positions[1], [100.0, 50.0, 0.0]);
        assert_eq!(p.positions[2], [100.0, 0.0, 50.0]);
        assert_eq!(p.indices, [0, 1, 2]);
        assert_eq!(p.texture.as_deref(), Some(tmp.path().join("tri.png").as_path()));
        assert!(load_gltf(&tmp.path().join("missing.gltf"), 50.0).is_err());
    }
}
