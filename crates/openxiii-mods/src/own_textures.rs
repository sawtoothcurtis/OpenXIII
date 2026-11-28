//! A mod's own textures: `[textures.<Name>]` tables, read from PNG files of the mod folder.
//!
//! ```toml
//! [textures.Poster]             # a new texture, named "Mod.Poster" wherever a texture is accepted
//! file = "textures/poster.png"
//!
//! [textures.Sun]
//! file = "textures/sun.png"
//! replace = "XIIIPlage.PLspot"  # also drawn instead of this game texture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use toml::Spanned;

use crate::placement::Source;
use crate::{line_of, Problem};

/// Prefix of the names of the mods' own textures.
pub const PREFIX: &str = "Mod.";

/// A `[textures.<Name>]` definition.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnTexture {
    /// Name as written ("Poster"); the texture is "Mod.Poster".
    pub name: String,
    /// PNG file, inside the mod folder.
    pub file: PathBuf,
    /// Game texture this one is drawn instead of, if any.
    pub replace: Option<String>,
    /// `clamp = true`: read at its edge outside the image instead of repeating (a mark projected
    /// on walls, as the game's bullet holes).
    pub clamp: bool,
    pub source: Source,
}

impl OwnTexture {
    /// Full texture name: "Mod.<Name>".
    pub fn texture_name(&self) -> String {
        format!("{PREFIX}{}", self.name)
    }
}

/// Mip levels, largest first: (width, height, RGBA bytes).
pub type Mips = Vec<(u32, u32, Vec<u8>)>;

pub(crate) type TextureTables = BTreeMap<String, Spanned<toml::Table>>;

/// Reads the `[textures.<Name>]` tables of a file of the mod in folder `mod_dir`. A wrong
/// table gives an error (with its line) and is skipped.
pub(crate) fn read(text: &str, file: &Path, mod_name: &str, mod_dir: &Path, tables: TextureTables, out: &mut Vec<OwnTexture>, problems: &mut Vec<Problem>) {
    for (name, t) in tables {
        let line = line_of(text, t.span().start);
        let err = |m: String| Problem::Error { file: file.to_path_buf(), line: Some(line), message: format!("textures.{name}: {m}; texture skipped") };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            problems.push(err("the name may only use letters, digits, _ and -".into()));
            continue;
        }
        let t = t.get_ref();
        if let Some(k) = t.keys().find(|k| !["file", "replace", "clamp"].contains(&k.as_str())) {
            problems.push(err(format!("unknown key {k} (keys: file, replace, clamp)")));
            continue;
        }
        let clamp = match t.get("clamp") {
            None => false,
            Some(toml::Value::Boolean(b)) => *b,
            Some(v) => {
                problems.push(err(format!("clamp = {v} must be true or false")));
                continue;
            }
        };
        let Some(toml::Value::String(f)) = t.get("file") else {
            problems.push(err("file = \"path inside the mod folder\" is missing".into()));
            continue;
        };
        let rel = Path::new(f);
        if crate::path_in(mod_dir, f).is_none() || rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            problems.push(err(format!("file {f:?} must be inside the mod folder")));
            continue;
        }
        let replace = match t.get("replace") {
            None => None,
            Some(toml::Value::String(r)) if r.contains('.') => Some(r.trim().to_string()),
            Some(v) => {
                problems.push(err(format!("replace = {v} must be a game texture \"Package.Group.Name\"")));
                continue;
            }
        };
        out.push(OwnTexture { name, file: crate::file_in_mod(mod_dir, f), replace, clamp, source: Source { mod_name: mod_name.to_string(), file: file.to_path_buf(), line } });
    }
}

/// Decodes a PNG file (any color type and bit depth) into RGBA, with its mip levels down to
/// 1×1 (each level the 2×2 average of the previous one).
pub fn decode_png(path: &Path) -> Result<Mips, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut d = png::Decoder::new(std::io::BufReader::new(f));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
    // Size checked from the header, before the pixels are decoded.
    let (w, h) = (r.info().width, r.info().height);
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return Err(format!("{}: {w}×{h}, sizes from 1 to 4096 are supported", path.display()));
    }
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = (info.width, info.height);
    let px = (w * h) as usize;
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..px * 4].to_vec(),
        png::ColorType::Rgb => buf[..px * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..px * 2].chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..px].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(format!("{}: palette not expanded", path.display())),
    };
    Ok(mips(w, h, rgba))
}

/// Mip levels of an RGBA image, largest first, down to 1×1.
pub fn mips(w: u32, h: u32, rgba: Vec<u8>) -> Mips {
    let mut out: Mips = vec![(w, h, rgba)];
    loop {
        let (pw, ph, prev) = out.last().unwrap();
        if *pw == 1 && *ph == 1 {
            break;
        }
        let (nw, nh) = ((pw / 2).max(1), (ph / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(pw - 1);
                        let sy = (y * 2 + dy).min(ph - 1);
                        sum += prev[((sy * pw + sx) * 4 + c) as usize] as u32;
                    }
                    next[((y * nw + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
                }
            }
        }
        out.push((nw, nh, next));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::parse_mod_file;

    #[test]
    fn tables_and_errors() {
        let text = "[textures.Poster]\nfile = \"textures/poster.png\"\n\n[textures.Sun]\nfile = \"sun.png\"\nreplace = \"XIIIPlage.PLspot\"\n\n[textures.Bad]\nfile = \"C:/elsewhere.png\"\n";
        let f = parse_mod_file(text, Path::new("m/mod.toml"), "m");
        assert_eq!(f.textures.len(), 2, "{:?}", f.textures);
        let poster = f.textures.iter().find(|t| t.name == "Poster").unwrap();
        assert_eq!(poster.texture_name(), "Mod.Poster");
        assert_eq!(poster.file, Path::new("m").join("textures/poster.png"));
        assert_eq!(f.textures.iter().find(|t| t.name == "Sun").unwrap().replace.as_deref(), Some("XIIIPlage.PLspot"));
        assert!(f.problems.iter().any(|p| p.to_string().contains("(line 8)")), "{:?}", f.problems);
    }

    #[test]
    fn clamp_key() {
        let text = "[textures.Burn]\nfile = \"burn.png\"\nclamp = true\n\n[textures.Plain]\nfile = \"plain.png\"\n\n[textures.Bad]\nfile = \"bad.png\"\nclamp = \"yes\"\n";
        let f = parse_mod_file(text, Path::new("m/mod.toml"), "m");
        assert!(f.textures.iter().find(|t| t.name == "Burn").unwrap().clamp);
        // Negative check: without the key, the texture repeats as before.
        assert!(!f.textures.iter().find(|t| t.name == "Plain").unwrap().clamp);
        assert!(f.problems.iter().any(|p| p.to_string().contains("clamp")), "{:?}", f.problems);
    }

    #[test]
    fn png_and_mips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.png");
        let (w, h) = (8u32, 4u32);
        let data: Vec<u8> = (0..w * h).flat_map(|i| [i as u8 * 8, 100, 200]).collect();
        {
            let f = std::fs::File::create(&path).unwrap();
            let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            e.write_header().unwrap().write_image_data(&data).unwrap();
        }
        let m = decode_png(&path).unwrap();
        let sizes: Vec<(u32, u32)> = m.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(sizes, [(8, 4), (4, 2), (2, 1), (1, 1)]);
        assert_eq!(&m[0].2[0..8], &[0, 100, 200, 255, 8, 100, 200, 255]);
        // First pixel of level 1: average of pixels 0, 1, 8, 9 (red 0, 8, 64, 72).
        assert_eq!(m[1].2[0], 36);
        assert!(m.iter().all(|(w, h, d)| d.len() == (w * h * 4) as usize));
        std::fs::write(tmp.path().join("bad.png"), b"nope").unwrap();
        assert!(decode_png(&tmp.path().join("bad.png")).is_err());
    }
}
