//! XIII fonts (Font class): character pages drawn into textures.
//!
//! After the properties: pages (compact n × [texture (compact object index), characters
//! (compact n × [StartU i32, StartV i32, USize i32, VSize i32])]), then CharactersPerPage (i32)
//! then, from package version 0x45, a byte table (compact n × [key, value]; UFont's TMap at +0x38) and an integer
//! (+0x4C) — a compact 0 and zeros in the game's fonts. Character c is in page c / CharactersPerPage.

use openxiii_package::{ObjectRef, Package, Reader, Result};

#[derive(Debug, Clone, Copy, Default)]
pub struct Glyph {
    pub u: i32,
    pub v: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone)]
pub struct FontPage {
    /// Page texture ("Package.Group.Texture").
    pub texture: String,
    pub glyphs: Vec<Glyph>,
}

#[derive(Debug, Clone, Default)]
pub struct Font {
    pub pages: Vec<FontPage>,
    pub characters_per_page: usize,
    /// UFont table at +0x38 (version >= 0x45): (key, value), in file order.
    pub table: Vec<(u8, u8)>,
    /// UFont integer at +0x4C (version >= 0x45).
    pub field_4c: i32,
}

/// Characters re-framed by UFont::Serialize when the language is "POT": (index, p2, p3, p4, p5), arguments.
const REFRAMES_POT: [(i32, f32, f32, f32, f32); 8] = [
    (0xE6, 0.2, -0.2, 0.0, 0.0),
    (0x9C, 0.2, -0.2, 0.0, 0.0),
    (0x8C, 0.3, -0.2, 0.0, 0.0),
    (0xA3, 0.0, -0.2, 0.05, 0.05),
    (0xB3, 0.0, -0.1, 0.0, 0.0),
    (0xC6, 0.2, -0.2, 0.0, 0.0),
    (0xAF, 0.0, 0.0, -0.025, -0.025),
    (0x74, 0.0, 0.0, 0.0, -0.025),
];

impl Font {
    /// `package_name`: package name (to qualify the textures exported in it). `language_value`: game language (GetLanguage).
    pub fn read(package_name: &str, package: &Package, data: &[u8], index: usize, language_value: &str) -> Result<Self> {
        let object = package.object_data(data, index)?;
        let export = &package.exports[index];
        let end = export.serial_offset as usize + export.serial_size as usize;
        let mut r = Reader::new(&data[..end], object.end);
        Self::read_from(&mut r, package.header.version, language_value, |t| match ObjectRef::from(t) {
            ObjectRef::None => String::new(),
            ObjectRef::Export(_) => format!("{package_name}.{}", package.object_path(t.into())),
            ObjectRef::Import(_) => package.object_path(t.into()),
        })
    }

    /// UFont::Serialize when loading, after the properties. `version`: package version.
    pub fn read_from(r: &mut Reader, version: u16, language_value: &str, texture_of: impl Fn(i32) -> String) -> Result<Self> {
        let mut pages = Vec::new();
        for _ in 0..r.compact()? {
            let t = r.compact()?;
            let texture = texture_of(t);
            let glyphs = (0..r.compact()?).map(|_| Ok(Glyph { u: r.i32()?, v: r.i32()?, width: r.i32()?, height: r.i32()? })).collect::<Result<Vec<_>>>()?;
            pages.push(FontPage { texture, glyphs });
        }
        let per_page = r.i32()?;
        let (mut table, mut field_4c) = (Vec::new(), 0);
        if version >= 0x45 {
            for _ in 0..r.compact()? {
                table.push((r.u8()?, r.u8()?));
            }
            field_4c = r.i32()?;
        }
        let mut font = Font { pages, characters_per_page: per_page.max(1) as usize, table, field_4c };
        if language_value.eq_ignore_ascii_case("pot") && per_page > 0 {
            for (hint, p2, p3, p4, p5) in REFRAMES_POT {
                font.reframe(hint, per_page, p2, p3, p4, p5);
            }
        }
        Ok(font)
    }

    /// Re-frames glyph `index` in fractions of its size (x87 in 24-bit precision: each operation rounded to
    /// f32, integer conversions by truncation). A missing glyph is left untouched (the original would read past the array).
    fn reframe(&mut self, hint: i32, by_page: i32, p2: f32, p3: f32, p4: f32, p5: f32) {
        let Some(g) = self.pages.get_mut((hint / by_page) as usize).and_then(|p| p.glyphs.get_mut((hint % by_page) as usize)) else { return };
        let (u, v, w, h) = (g.u, g.v, g.width as f32, g.height as f32);
        g.u = ((w * p2) + u as f32) as i32;
        g.v = ((h * p4) + v as f32) as i32;
        g.width = (((p3 - p2) + 1.0) * w) as i32;
        g.height = (((p5 - p4) + 1.0) * h) as i32;
    }

    /// Page and drawing of character `c` (None if it does not exist or is empty).
    pub fn glyph(&self, c: char) -> Option<(&FontPage, Glyph)> {
        let code = c as usize;
        let page = self.pages.get(code / self.characters_per_page)?;
        let g = *page.glyphs.get(code % self.characters_per_page)?;
        (g.width > 0 || g.height > 0).then_some((page, g))
    }

    /// Line height (the tallest character).
    pub fn line_height(&self) -> i32 {
        self.pages.iter().flat_map(|p| p.glyphs.iter()).map(|g| g.height).max().unwrap_or(16)
    }

    /// Size of a text (pixels, scale 1).
    pub fn text_size(&self, text: &str) -> (f32, f32) {
        let width: i32 = text.chars().map(|c| self.glyph(c).map(|(_, g)| g.width).unwrap_or(0)).sum();
        (width as f32, self.line_height() as f32)
    }
}

