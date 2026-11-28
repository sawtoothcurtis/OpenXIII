//! Other arms in first person: `[arms.<Name>]` tables, a recipe applied to the first-person meshes
//! of the player's game when it starts (nothing of the game is copied into the mod).
//!
//! ```toml
//! [arms.Jones]
//! from = ["XIIIArmes.*", "XIIIDeco.*"]  # every first-person mesh of these packages, or one mesh
//!                                       # ("XIIIArmes.FpsBerrettaM")
//! replace = true                    # drawn instead of the game's meshes (otherwise only under
//!                                   # "Mod.Jones.<Mesh>", and "Mod.Jones" for a single mesh)
//! skip = ["XIIIArmes.FpsCadavreM"]  # meshes left as they are (optional)
//! fingers = 0.8                     # thickness of the fingers (1 = unchanged)
//! hand = 0.85                       # thickness of the hands
//! wrist = 0.8                       # thickness of the wrists
//! forearm = 0.9                     # thickness of the forearms at the elbow
//! skin_from = "XIIIPersos.JonesM"   # skin colour measured on this character's hands and forearms
//! # skin = [165, 100, 60]           # or a colour (red, green, blue)
//! nails = [215, 160, 150]           # colour of the nails (optional)
//!
//! [arms.Jones.mesh."XIIIArmes.FpsCanneM"]   # other thicknesses for one mesh (optional)
//! fingers = 0.9
//!
//! [arms.Carrington.sleeve]          # a sleeve over the forearm (optional; bare arms without it)
//! fabric_from = "XIIIPersos.CarringtonM"  # colour measured on this character's arms
//! # fabric = [90, 93, 66]           # or a colour
//! cuff = 0.9                        # where it ends: 0 at the elbow, 1 at the wrist
//! band = [60, 62, 44]               # a band at its edge (optional)
//! band_width = 0.05                 # its width along the forearm (0.05 by default)
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use toml::Spanned;

use crate::placement::Source;
use crate::{line_of, Problem};

/// The skin tone asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Skin {
    Unchanged,
    Color([u8; 3]),
    /// A character model of the game.
    From(String),
}

/// Thickness factors: fingers, hand, wrist, forearm (1 = unchanged).
pub type Factors = [f32; 4];

/// An `[arms.<Name>]` definition.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnArms {
    /// Name as written ("Jones"); the meshes are "Mod.Jones.<Mesh>" ("Mod.Jones" for one mesh).
    pub name: String,
    /// Game meshes, or "Package.*" for every first-person mesh of a package.
    pub from: Vec<String>,
    /// Drawn instead of the game's meshes.
    pub replace: bool,
    pub skip: Vec<String>,
    pub factors: Factors,
    /// Other factors for some meshes.
    pub per_mesh: Vec<(String, Factors)>,
    /// Another place for the sleeve's cuff on some meshes (`cuff` in a mesh table).
    pub per_mesh_cuff: Vec<(String, f32)>,
    pub skin: Skin,
    pub nails: Option<[u8; 3]>,
    pub sleeve: Option<OwnSleeve>,
    /// Gloves (`[arms.<Name>.gloves]`): their colour and where they start along the forearm.
    pub gloves: Option<(Skin, f32, Option<f32>)>,
    /// An arm not drawn (`hide_arm = "left"` or `"right"`): true for the left one.
    pub hide_left: Option<bool>,
    pub source: Source,
}

/// An `[arms.<Name>.sleeve]` table.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnSleeve {
    /// The fabric's colour (`Skin::From`: measured on a character model's arms).
    pub fabric: Skin,
    pub cuff: f32,
    /// Band colour and width.
    pub band: Option<([u8; 3], f32)>,
    /// Where the band starts along the forearm (None: it ends at the cuff).
    pub band_at: Option<f32>,
    /// A couple of thin crease lines, as the game's ink.
    pub creases: bool,
    /// Thin ink lines all around the sleeve (along the forearm).
    pub rings: Vec<f32>,
    /// The rings' colour and width instead of ink.
    pub ring_colour: Option<([u8; 3], f32)>,
    /// A strap tab: from, to, angle, half width.
    pub tab: Option<[f32; 4]>,
    /// A ribbed cuff: its stripes' colour, and from, how many around, width of each.
    pub ribs: Option<([u8; 3], [f32; 3])>,
    /// A check pattern: its lines' colour, and spacing along, lines around, width of each.
    pub checks: Option<([u8; 3], [f32; 3])>,
    /// Buttons near the cuff: their colour, and where the first is, how many, angle, spacing.
    pub buttons: Option<([u8; 3], [f32; 4])>,
    /// A sleeve in relief.
    pub model: Option<OwnRelief>,
}

/// The `relief_*` keys of a sleeve table.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnRelief {
    /// The character model whose sleeve is used (its texture, unless `painted`).
    pub from: String,
    pub thickness: f32,
    pub room: f32,
    pub lip: f32,
    pub seam: f32,
    /// Drawn with the painted sleeve (plain, in the fabric's colour).
    pub painted: bool,
    pub folds: usize,
    pub fold_depth: f32,
    pub fold_width: f32,
    pub fold_slant: f32,
    /// Narrower towards the cuff: from, factor at the cuff.
    pub taper: Option<(f32, f32)>,
}

impl OwnArms {
    /// "Mod.<Name>".
    pub fn mesh_name(&self) -> String {
        format!("{}{}", crate::own_models::PREFIX, self.name)
    }
}

pub(crate) type ArmsTables = BTreeMap<String, Spanned<toml::Table>>;

const KEYS: [&str; 14] = ["from", "replace", "skip", "mesh", "sleeve", "gloves", "fingers", "hand", "wrist", "forearm", "skin", "skin_from", "nails", "hide_arm"];
const SLEEVE_KEYS: [&str; 28] = [
    "fabric", "fabric_from", "cuff", "band", "band_width", "band_at", "creases", "rings", "ring_colour", "ring_width", "tab", "ribs", "rib_colour", "checks", "check_colour", "buttons", "button_colour", "relief_from", "relief_texture", "thickness", "room", "lip", "seam", "folds", "fold_depth", "fold_width", "fold_slant", "taper",
];

/// An `[arms.<Name>.gloves]` table: `colour = [r, g, b]` or `colour_from = "Package.Mesh"` (a
/// character model's hands), and `from` (where they start along the forearm, 0.98 by default).
fn gloves(v: &toml::Value) -> Result<(Skin, f32, Option<f32>), String> {
    let toml::Value::Table(t) = v else { return Err("gloves must be a table [arms.<Name>.gloves]".into()) };
    if let Some(k) = t.keys().find(|k| !["colour", "colour_from", "from", "to"].contains(&k.as_str())) {
        return Err(format!("gloves: unknown key {k} (keys: colour, colour_from, from, to)"));
    }
    let colour = match (t.get("colour"), t.get("colour_from")) {
        (Some(_), Some(_)) => return Err("gloves: give colour or colour_from, not both".into()),
        (Some(v), None) => Skin::Color(color(v).ok_or_else(|| format!("gloves: colour = {v} must be [red, green, blue] (0 to 255)"))?),
        (None, Some(toml::Value::String(m))) if m.contains('.') => Skin::From(m.trim().to_string()),
        (None, Some(v)) => return Err(format!("gloves: colour_from = {v} must be a character model \"Package.Mesh\"")),
        (None, None) => return Err("gloves: colour = [r, g, b] or colour_from = \"Package.Mesh\" is missing".into()),
    };
    let from = match t.get("from") {
        None => 0.98,
        Some(v) => match v.as_float().or_else(|| v.as_integer().map(|i| i as f64)) {
            Some(f) if (0.0..=1.5).contains(&f) => f as f32,
            _ => return Err(format!("gloves: from = {v} must be a number from 0 to 1.5")),
        },
    };
    let to = match t.get("to") {
        None => None,
        Some(v) => match v.as_float().or_else(|| v.as_integer().map(|i| i as f64)) {
            Some(f) if (0.0..=3.0).contains(&f) && f as f32 > from => Some(f as f32),
            _ => return Err(format!("gloves: to = {v} must be a number after from, up to 3")),
        },
    };
    Ok((colour, from, to))
}

fn sleeve(v: &toml::Value) -> Result<OwnSleeve, String> {
    let toml::Value::Table(t) = v else { return Err("sleeve must be a table [arms.<Name>.sleeve]".into()) };
    if let Some(k) = t.keys().find(|k| !SLEEVE_KEYS.contains(&k.as_str())) {
        return Err(format!("sleeve: unknown key {k} (keys: {})", SLEEVE_KEYS.join(", ")));
    }
    let fabric = match (t.get("fabric"), t.get("fabric_from")) {
        (Some(_), Some(_)) => return Err("sleeve: give fabric or fabric_from, not both".into()),
        (Some(v), None) => Skin::Color(color(v).ok_or_else(|| format!("sleeve: fabric = {v} must be [red, green, blue] (0 to 255)"))?),
        (None, Some(toml::Value::String(m))) if m.contains('.') => Skin::From(m.trim().to_string()),
        (None, Some(v)) => return Err(format!("sleeve: fabric_from = {v} must be a character model \"Package.Mesh\"")),
        (None, None) => return Err("sleeve: fabric = [r, g, b] or fabric_from = \"Package.Mesh\" is missing".into()),
    };
    let number = |key: &str, default: f64| -> Result<f32, String> {
        match t.get(key) {
            None => Ok(default as f32),
            Some(v) => match v.as_float().or_else(|| v.as_integer().map(|i| i as f64)) {
                Some(f) if (0.0..=1.0).contains(&f) => Ok(f as f32),
                _ => Err(format!("sleeve: {key} = {v} must be a number from 0 to 1")),
            },
        }
    };
    let cuff = number("cuff", 0.9)?;
    let width = number("band_width", 0.05)?;
    let band = t.get("band").map(|v| color(v).ok_or_else(|| format!("sleeve: band = {v} must be [red, green, blue] (0 to 255)"))).transpose()?.map(|c| (c, width));
    let ranged = |key: &str, default: f64, lo: f64, hi: f64| -> Result<f32, String> {
        match t.get(key) {
            None => Ok(default as f32),
            Some(v) => match v.as_float().or_else(|| v.as_integer().map(|i| i as f64)) {
                Some(f) if (lo..=hi).contains(&f) => Ok(f as f32),
                _ => Err(format!("sleeve: {key} = {v} must be a number from {lo} to {hi}")),
            },
        }
    };
    let model = match t.get("relief_from") {
        None => None,
        Some(toml::Value::String(m)) if m.contains('.') => {
            let painted = match t.get("relief_texture") {
                None => true,
                Some(toml::Value::String(s)) if s == "painted" => true,
                Some(toml::Value::String(s)) if s == "model" => false,
                Some(v) => return Err(format!("sleeve: relief_texture = {v} must be \"painted\" or \"model\"")),
            };
            let folds = match t.get("folds") {
                None => 3,
                Some(v) => v.as_integer().filter(|n| (0..=8).contains(n)).ok_or_else(|| format!("sleeve: folds = {v} must be a whole number from 0 to 8"))? as usize,
            };
            Some(OwnRelief {
                from: m.trim().to_string(),
                thickness: ranged("thickness", 1.1, 1.0, 2.0)?,
                room: ranged("room", 0.3, 0.0, 5.0)?,
                lip: ranged("lip", 1.08, 1.0, 1.5)?,
                seam: ranged("seam", 0.0, 0.0, 1.0)?,
                painted,
                folds,
                fold_depth: ranged("fold_depth", 0.08, 0.0, 0.5)?,
                fold_width: ranged("fold_width", 0.07, 0.01, 0.5)?,
                fold_slant: ranged("fold_slant", 0.05, 0.0, 0.5)?,
                taper: match t.get("taper") {
                    None => None,
                    Some(v) => match v.as_array().map(|a| a.iter().filter_map(|x| x.as_float().or_else(|| x.as_integer().map(|i| i as f64))).collect::<Vec<f64>>()) {
                        Some(a) if a.len() == 2 && (0.0..=1.0).contains(&a[0]) && (0.5..=1.0).contains(&a[1]) => Some((a[0] as f32, a[1] as f32)),
                        _ => return Err(format!("sleeve: taper = {v} must be [from (0 to 1), factor at the cuff (0.5 to 1)]")),
                    },
                },
            })
        }
        Some(v) => return Err(format!("sleeve: relief_from = {v} must be a character model \"Package.Mesh\"")),
    };
    let band_at = match t.get("band_at") {
        None => None,
        Some(_) => Some(number("band_at", 0.0)?),
    };
    let creases = match t.get("creases") {
        None => false,
        Some(toml::Value::Boolean(b)) => *b,
        Some(v) => return Err(format!("sleeve: creases = {v} must be true or false")),
    };
    let numbers = |key: &str, n: Option<usize>| -> Result<Vec<f32>, String> {
        let Some(v) = t.get(key) else { return Ok(Vec::new()) };
        let list: Vec<f32> = v.as_array().map(|a| a.iter().filter_map(|x| x.as_float().or_else(|| x.as_integer().map(|i| i as f64))).map(|f| f as f32).collect()).unwrap_or_default();
        if v.as_array().map_or(true, |a| a.len() != list.len()) || n.is_some_and(|n| list.len() != n) {
            return Err(format!("sleeve: {key} = {v} must be a list of numbers"));
        }
        Ok(list)
    };
    let rings = numbers("rings", None)?;
    let tab = match numbers("tab", Some(4)) {
        Ok(v) if v.is_empty() => None,
        Ok(v) => Some([v[0], v[1], v[2], v[3]]),
        Err(_) => return Err("sleeve: tab must be [from, to, angle, half width]".into()),
    };
    let ribs = match (numbers("ribs", Some(3)), t.get("rib_colour")) {
        (Ok(v), None) if v.is_empty() => None,
        (Ok(v), Some(c)) if v.len() == 3 && (0.0..=1.0).contains(&v[0]) && (1.0..=64.0).contains(&v[1]) && v[1].fract() == 0.0 && v[2] > 0.0 && v[2] < 1.0 => {
            Some((color(c).ok_or_else(|| format!("sleeve: rib_colour = {c} must be [red, green, blue] (0 to 255)"))?, [v[0], v[1], v[2]]))
        }
        (Ok(v), None) if !v.is_empty() => return Err("sleeve: ribs needs rib_colour = [red, green, blue]".into()),
        _ => return Err("sleeve: ribs must be [from (0 to 1), how many around (1 to 64), width of each (0 to 1)] with rib_colour".into()),
    };
    let with_colour = |key: &str, colour_key: &str, n: usize, check: &dyn Fn(&[f32]) -> bool, what: &str| -> Result<Option<([u8; 3], Vec<f32>)>, String> {
        match (numbers(key, Some(n)), t.get(colour_key)) {
            (Ok(v), None) if v.is_empty() => Ok(None),
            (Ok(v), Some(c)) if v.len() == n && check(&v) => Ok(Some((color(c).ok_or_else(|| format!("sleeve: {colour_key} = {c} must be [red, green, blue] (0 to 255)"))?, v))),
            (Ok(v), None) if !v.is_empty() => Err(format!("sleeve: {key} needs {colour_key} = [red, green, blue]")),
            _ => Err(format!("sleeve: {key} must be {what} with {colour_key}")),
        }
    };
    let checks = with_colour("checks", "check_colour", 3, &|v| v[0] > 0.0 && v[0] <= 2.0 && (1.0..=64.0).contains(&v[1]) && v[1].fract() == 0.0 && v[2] > 0.0 && v[2] < 0.5, "[spacing along (0 to 2), lines around (1 to 64), width of each (0 to 0.5)]")?
        .map(|(c, v)| (c, [v[0], v[1], v[2]]));
    let buttons = with_colour("buttons", "button_colour", 4, &|v| (0.0..=1.0).contains(&v[0]) && (1.0..=8.0).contains(&v[1]) && v[1].fract() == 0.0 && v[3] > 0.0 && v[3] <= 0.3, "[first along the forearm (0 to 1), how many (1 to 8), angle, spacing (0 to 0.3)]")?
        .map(|(c, v)| (c, [v[0], v[1], v[2], v[3]]));
    let ring_colour = match t.get("ring_colour") {
        None if t.contains_key("ring_width") => return Err("sleeve: ring_width needs ring_colour".into()),
        None => None,
        Some(v) => Some((color(v).ok_or_else(|| format!("sleeve: ring_colour = {v} must be [red, green, blue] (0 to 255)"))?, number("ring_width", 0.012)?)),
    };
    Ok(OwnSleeve { fabric, cuff, band, band_at, creases, rings, ring_colour, tab, ribs, checks, buttons, model })
}
const FACTORS: [&str; 4] = ["fingers", "hand", "wrist", "forearm"];

fn color(v: &toml::Value) -> Option<[u8; 3]> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    let c: Vec<u8> = a.iter().filter_map(|x| x.as_integer()).filter_map(|x| u8::try_from(x).ok()).collect();
    (c.len() == 3).then(|| [c[0], c[1], c[2]])
}

/// The thickness factors of a table, starting from `base`.
fn factors(t: &toml::Table, base: Factors) -> Result<Factors, String> {
    let mut out = base;
    for (k, key) in FACTORS.iter().enumerate() {
        if let Some(v) = t.get(*key) {
            match v.as_float().or_else(|| v.as_integer().map(|i| i as f64)) {
                Some(f) if (0.3..=1.5).contains(&f) => out[k] = f as f32,
                _ => return Err(format!("{key} = {v} must be a number from 0.3 to 1.5")),
            }
        }
    }
    Ok(out)
}

/// A mesh name "Package.Mesh" or, when `wildcard`, "Package.*".
fn mesh_names(v: &toml::Value, key: &str, wildcard: bool) -> Result<Vec<String>, String> {
    let list: Vec<&toml::Value> = match v {
        toml::Value::Array(a) => a.iter().collect(),
        other => vec![other],
    };
    let what = if wildcard { "\"Package.Mesh\" or \"Package.*\"" } else { "\"Package.Mesh\"" };
    let names: Vec<String> = list.iter().filter_map(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| s.contains('.') && (wildcard || !s.ends_with(".*"))).collect();
    if names.len() != list.len() || (names.is_empty() && key == "from") {
        return Err(format!("{key} = {v} must be {what} or a list of them"));
    }
    Ok(names)
}

/// Reads the `[arms.<Name>]` tables of a file. A wrong table gives an error (with its line) and
/// is skipped.
pub(crate) fn read(text: &str, file: &Path, mod_name: &str, tables: ArmsTables, out: &mut Vec<OwnArms>, problems: &mut Vec<Problem>) {
    for (name, t) in tables {
        let line = line_of(text, t.span().start);
        let err = |m: String| Problem::Error { file: file.to_path_buf(), line: Some(line), message: format!("arms.{name}: {m}; arms skipped") };
        match parse(&name, t.get_ref()) {
            Ok(mut a) => {
                a.source = Source { mod_name: mod_name.to_string(), file: file.to_path_buf(), line };
                out.push(a);
            }
            Err(m) => problems.push(err(m)),
        }
    }
}

fn parse(name: &str, t: &toml::Table) -> Result<OwnArms, String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("the name may only use letters, digits, _ and -".into());
    }
    if let Some(k) = t.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key {k} (keys: {})", KEYS.join(", ")));
    }
    let from = mesh_names(t.get("from").ok_or("from = \"Package.Mesh\" or \"Package.*\" (the game's first-person meshes) is missing")?, "from", true)?;
    let skip = t.get("skip").map(|v| mesh_names(v, "skip", false)).transpose()?.unwrap_or_default();
    let replace = match t.get("replace") {
        None => false,
        Some(toml::Value::Boolean(b)) => *b,
        Some(v) => return Err(format!("replace = {v} must be true or false")),
    };
    let base = factors(t, [1.0; 4])?;
    let mut per_mesh = Vec::new();
    let mut per_mesh_cuff = Vec::new();
    match t.get("mesh") {
        None => {}
        Some(toml::Value::Table(m)) => {
            for (mesh, v) in m {
                let toml::Value::Table(mt) = v else { return Err(format!("mesh.\"{mesh}\" must be a table of thicknesses")) };
                if let Some(k) = mt.keys().find(|k| !FACTORS.contains(&k.as_str()) && k.as_str() != "cuff") {
                    return Err(format!("mesh.\"{mesh}\": unknown key {k} (keys: {}, cuff)", FACTORS.join(", ")));
                }
                if let Some(v) = mt.get("cuff") {
                    match v.as_float().or_else(|| v.as_integer().map(|i| i as f64)) {
                        Some(f) if (0.0..=1.0).contains(&f) && t.get("sleeve").is_some() => per_mesh_cuff.push((mesh.clone(), f as f32)),
                        Some(f) if (0.0..=1.0).contains(&f) => return Err(format!("mesh.\"{mesh}\": cuff needs a sleeve [arms.{name}.sleeve]")),
                        _ => return Err(format!("mesh.\"{mesh}\": cuff = {v} must be a number from 0 to 1")),
                    }
                }
                if !mesh.contains('.') {
                    return Err(format!("mesh.\"{mesh}\" must name a mesh \"Package.Mesh\""));
                }
                per_mesh.push((mesh.clone(), factors(mt, base).map_err(|e| format!("mesh.\"{mesh}\": {e}"))?));
            }
        }
        Some(v) => return Err(format!("mesh = {v} must be tables [arms.{name}.mesh.\"Package.Mesh\"]")),
    }
    let skin = match (t.get("skin"), t.get("skin_from")) {
        (Some(_), Some(_)) => return Err("give skin or skin_from, not both".into()),
        (Some(v), None) => Skin::Color(color(v).ok_or_else(|| format!("skin = {v} must be [red, green, blue] (0 to 255)"))?),
        (None, Some(toml::Value::String(m))) if m.contains('.') => Skin::From(m.trim().to_string()),
        (None, Some(v)) => return Err(format!("skin_from = {v} must be a character model \"Package.Mesh\"")),
        (None, None) => Skin::Unchanged,
    };
    let nails = t.get("nails").map(|v| color(v).ok_or_else(|| format!("nails = {v} must be [red, green, blue] (0 to 255)"))).transpose()?;
    let sleeve = t.get("sleeve").map(sleeve).transpose()?;
    let gloves = t.get("gloves").map(gloves).transpose()?;
    let hide_left = match t.get("hide_arm") {
        None => None,
        Some(toml::Value::String(s)) if s == "left" => Some(true),
        Some(toml::Value::String(s)) if s == "right" => Some(false),
        Some(v) => return Err(format!("hide_arm = {v} must be \"left\" or \"right\"")),
    };
    Ok(OwnArms {
        name: name.to_string(),
        from,
        replace,
        skip,
        factors: base,
        per_mesh,
        per_mesh_cuff,
        skin,
        nails,
        sleeve,
        gloves,
        hide_left,
        source: Source { mod_name: String::new(), file: Default::default(), line: 0 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_file(text: &str) -> crate::defaults::ModFile {
        crate::defaults::parse_mod_file(text, Path::new("mod.toml"), "m")
    }

    #[test]
    fn reads_a_recipe() {
        let f = parse_file("[arms.Jones]\nfrom = \"XIIIArmes.FpsBerrettaM\"\nfingers = 0.8\nhand = 1\nskin_from = \"XIIIPersos.JonesM\"\nnails = [215, 160, 150]\n");
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let a = &f.arms[0];
        assert_eq!(a.mesh_name(), "Mod.Jones");
        assert_eq!(a.from, vec!["XIIIArmes.FpsBerrettaM".to_string()]);
        assert!(!a.replace);
        assert_eq!(a.factors, [0.8, 1.0, 1.0, 1.0]);
        assert_eq!(a.skin, Skin::From("XIIIPersos.JonesM".into()));
        assert_eq!(a.nails, Some([215, 160, 150]));
        assert_eq!(a.source.line, 1);
    }

    #[test]
    fn reads_packages_skips_and_mesh_tables() {
        let f = parse_file(
            "[arms.J]\nfrom = [\"XIIIArmes.*\", \"XIIIDeco.*\"]\nreplace = true\nskip = [\"XIIIArmes.FpsCadavreM\"]\nfingers = 0.7\n\n[arms.J.mesh.\"XIIIArmes.FpsCanneM\"]\nhand = 0.9\n",
        );
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let a = &f.arms[0];
        assert_eq!(a.from.len(), 2);
        assert!(a.replace);
        assert_eq!(a.skip, vec!["XIIIArmes.FpsCadavreM".to_string()]);
        // The mesh's table starts from the arms' own factors.
        assert_eq!(a.per_mesh, vec![("XIIIArmes.FpsCanneM".to_string(), [0.7, 0.9, 1.0, 1.0])]);
    }

    #[test]
    fn reads_a_sleeve() {
        let f = parse_file("[arms.C]\nfrom = \"XIIIArmes.*\"\n[arms.C.sleeve]\nfabric_from = \"XIIIPersos.CarringtonM\"\ncuff = 0.85\nband = [1, 2, 3]\n");
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let s = f.arms[0].sleeve.clone().unwrap();
        assert_eq!(s.fabric, Skin::From("XIIIPersos.CarringtonM".into()));
        assert_eq!(s.cuff, 0.85);
        assert_eq!(s.band, Some(([1, 2, 3], 0.05)));
        assert_eq!(s.ribs, None);
        // A ribbed cuff.
        let f = parse_file("[arms.K]\nfrom = \"X.Y\"\n[arms.K.sleeve]\nfabric = [1, 2, 3]\nribs = [0.43, 12, 0.55]\nrib_colour = [99, 130, 123]\n");
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        assert_eq!(f.arms[0].sleeve.as_ref().unwrap().ribs, Some(([99, 130, 123], [0.43, 12.0, 0.55])));
        // A tapered sleeve, its cuff moved on one mesh.
        let f = parse_file("[arms.K]\nfrom = \"X.*\"\n[arms.K.sleeve]\nfabric = [1, 2, 3]\nrelief_from = \"P.M\"\ntaper = [0.4, 0.7]\n[arms.K.mesh.\"X.Long\"]\ncuff = 0.85\n");
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        assert_eq!(f.arms[0].sleeve.as_ref().unwrap().model.as_ref().unwrap().taper, Some((0.4, 0.7)));
        assert_eq!(f.arms[0].per_mesh_cuff, vec![("X.Long".to_string(), 0.85)]);
        // One arm hidden, a plaid with buttons.
        let f = parse_file("[arms.A]\nfrom = \"X.*\"\nhide_arm = \"left\"\n[arms.A.sleeve]\nfabric = [1, 2, 3]\nchecks = [0.3, 4, 0.05]\ncheck_colour = [4, 5, 6]\nbuttons = [0.9, 3, 1.15, 0.045]\nbutton_colour = [7, 8, 9]\n");
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        assert_eq!(f.arms[0].hide_left, Some(true));
        let s = f.arms[0].sleeve.as_ref().unwrap();
        assert_eq!(s.checks, Some(([4, 5, 6], [0.3, 4.0, 0.05])));
        assert_eq!(s.buttons, Some(([7, 8, 9], [0.9, 3.0, 1.15, 0.045])));
        // No sleeve: bare arms.
        assert!(parse_file("[arms.J]\nfrom = \"X.Y\"\n").arms[0].sleeve.is_none());
    }

    #[test]
    fn wrong_values_skip_the_table() {
        for text in [
            "[arms.A]\nfingers = 0.8\n",
            "[arms.A]\nfrom = \"X.Y\"\nfingers = 0.1\n",
            "[arms.A]\nfrom = \"X.Y\"\nskin = [1, 2]\n",
            "[arms.A]\nfrom = \"X.Y\"\nskin = [1, 2, 3]\nskin_from = \"P.M\"\n",
            "[arms.A]\nfrom = \"X.Y\"\ncolour = 1\n",
            "[arms.A]\nfrom = \"X.Y\"\nreplace = 1\n",
            "[arms.A]\nfrom = \"X.Y\"\nskip = [\"X.*\"]\n",
            "[arms.A]\nfrom = \"X.Y\"\n[arms.A.mesh.\"X.Z\"]\nskin = [1, 2, 3]\n",
            "[arms.A]\nfrom = \"X.Y\"\n[arms.A.sleeve]\ncuff = 0.5\n",
            "[arms.A]\nfrom = \"X.Y\"\n[arms.A.sleeve]\nfabric = [1, 2, 3]\ncuff = 2\n",
            "[arms.A]\nfrom = \"X.Y\"\n[arms.A.sleeve]\nfabric = [1, 2, 3]\nribs = [0.4, 12, 0.5]\n",
            "[arms.A]\nfrom = \"X.Y\"\n[arms.A.sleeve]\nfabric = [1, 2, 3]\nribs = [0.4, 12.5, 0.5]\nrib_colour = [1, 2, 3]\n",
            "[arms.A]\nfrom = \"X.Y\"\n[arms.A.sleeve]\nfabric = [1, 2, 3]\nrib_colour = [1, 2, 3]\n",
            "[arms.A]\nfrom = \"X.Y\"\nhide_arm = \"both\"\n",
            "[arms.A]\nfrom = \"X.Y\"\n[arms.A.sleeve]\nfabric = [1, 2, 3]\nbuttons = [0.9, 3, 1.0, 0.04]\n",
            "[arms.A]\nfrom = \"X.Y\"\n[arms.A.sleeve]\nfabric = [1, 2, 3]\nchecks = [0.3, 4.5, 0.05]\ncheck_colour = [1, 2, 3]\n",
            "[arms.A]\nfrom = \"X.Y\"\n[arms.A.mesh.\"X.Z\"]\ncuff = 0.8\n",
            "[arms.A]\nfrom = \"X.Y\"\n[arms.A.sleeve]\nfabric = [1, 2, 3]\nrelief_from = \"P.M\"\ntaper = [0.4, 0.2]\n",
        ] {
            let f = parse_file(text);
            assert!(f.arms.is_empty(), "{text}");
            assert_eq!(f.problems.len(), 1, "{text}");
        }
    }
}
