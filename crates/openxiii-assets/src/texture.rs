//! UE2 textures (Texture class): mip levels and decoding to 8-bit RGBA.
//!
//! After the properties: TArray<FMipmap> (compact count), each mip being
//! u32 end position, compact size + data, USize/VSize i32, UBits/VBits u8.
//! XIII adds 3 bytes after the array (unidentified); for P8 textures
//! without a Palette object, the palette (TArray<FColor>) is embedded before these 3 bytes.

use openxiii_package::{ObjectData, ObjectRef, Package, Reader, Value};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    P8,
    Rgba7,
    Rgb16,
    Dxt1,
    Rgb8,
    Rgba8,
    NoData,
    Dxt3,
    Dxt5,
    L8,
    G16,
    Rrrgggbbb,
    Unknown(u8),
}

impl From<u8> for Format {
    fn from(v: u8) -> Self {
        use Format::*;
        match v {
            0 => P8,
            1 => Rgba7,
            2 => Rgb16,
            3 => Dxt1,
            4 => Rgb8,
            5 => Rgba8,
            6 => NoData,
            7 => Dxt3,
            8 => Dxt5,
            9 => L8,
            10 => G16,
            11 => Rrrgggbbb,
            n => Unknown(n),
        }
    }
}

#[derive(Debug)]
pub enum TextureError {
    Package(openxiii_package::Error),
    Unsupported(Format),
    NoMips,
    MissingPalette,
    BadSize { expected: usize, got: usize },
}

impl fmt::Display for TextureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextureError::Package(e) => write!(f, "{e}"),
            TextureError::Unsupported(fmt) => write!(f, "format not supported: {fmt:?}"),
            TextureError::NoMips => write!(f, "no mip level"),
            TextureError::MissingPalette => write!(f, "palette missing or external"),
            TextureError::BadSize { expected, got } => {
                write!(f, "wrong mip size: {got} bytes, {expected} expected")
            }
        }
    }
}

impl std::error::Error for TextureError {}

impl From<openxiii_package::Error> for TextureError {
    fn from(e: openxiii_package::Error) -> Self {
        TextureError::Package(e)
    }
}

#[derive(Debug, Clone)]
pub struct Mip {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub format: Format,
    pub mips: Vec<Mip>,
    pub palette: Option<Vec<[u8; 4]>>,
    /// Bytes remaining after the mip array.
    pub trailing: Vec<u8>,
}

fn byte_prop(package: &Package, o: &ObjectData, name: &str) -> Option<u8> {
    match o.get(package, name)? {
        Value::Byte(b) => Some(*b),
        _ => None,
    }
}

impl Texture {
    pub fn read(package: &Package, data: &[u8], index: usize) -> Result<Self, TextureError> {
        let mut ch = openxiii_package::loadings::start_value(openxiii_package::loadings::TEXTURE_READ);
        ch.name_value(|| package.object_path(openxiii_package::ObjectRef::Export(index)));
        let object = package.object_data(data, index)?;
        let export = &package.exports[index];
        let end = export.serial_offset as usize + export.serial_size as usize;
        let mut r = Reader::new(&data[..end], object.end);
        let count = r.compact()?;
        let format = Format::from(byte_prop(package, &object, "Format").unwrap_or(0));
        let parametric = matches!(object.get(package, "bParametric"), Some(Value::Bool(true)));
        let mut mips = Vec::new();
        for _ in 0..count.max(0) {
            let _skip = r.u32()?;
            let size = r.compact()?;
            let bytes = r.take(size.max(0) as usize)?.to_vec();
            let width = r.i32()? as u32;
            let height = r.i32()? as u32;
            let _ubits = r.u8()?;
            let _vbits = r.u8()?;
            // Empty then AddZeroed for bParametric.
            let data = if parametric {
                let count = width.wrapping_mul(height).wrapping_mul(if format == Format::Rgba8 { 4 } else { 1 });
                vec![0; count as usize]
            } else { bytes };
            mips.push(Mip { data, width, height });
        }
        let trailing = data[r.pos..end].to_vec();
        let mut palette = match object.get(package, "Palette") {
            Some(Value::Object(o)) => match ObjectRef::from(*o) {
                ObjectRef::Export(i) => Some(read_palette(package, data, i)?),
                _ => None,
            },
            _ => None,
        };
        // Formats 0/13, LicenseeVer >= 42, array even if empty.
        if palette.is_none()
            && matches!(format, Format::P8 | Format::Unknown(13))
            && package.header.licensee >= 0x2A
            && !trailing.is_empty()
        {
            let mut t = Reader::new(&trailing, 0);
            let count = t.compact()?.max(0) as usize;
            let colors = t.take(count * 4)?;
            palette = Some(colors.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect());
        }
        Ok(Texture { format, mips, palette, trailing })
    }

    /// Decodes mip `level` to 8-bit RGBA.
    pub fn decode(&self, level: usize) -> Result<(u32, u32, Vec<u8>), TextureError> {
        let mip = self.mips.get(level).ok_or(TextureError::NoMips)?;
        let (w, h) = (mip.width as usize, mip.height as usize);
        let check = |expected: usize| {
            if mip.data.len() < expected {
                Err(TextureError::BadSize { expected, got: mip.data.len() })
            } else {
                Ok(())
            }
        };
        let blocks = w.div_ceil(4) * h.div_ceil(4);
        let rgba = match self.format {
            Format::P8 => {
                check(w * h)?;
                let palette = self.palette.as_ref().ok_or(TextureError::MissingPalette)?;
                mip.data[..w * h].iter().flat_map(|&i| palette.get(i as usize).copied().unwrap_or([0, 0, 0, 255])).collect()
            }
            Format::Unknown(13) => {
                // palette16, low then high nibble, source step w >> 1.
                let stride = w / 2;
                check(if h == 0 { 0 } else { (h - 1) * stride + w.div_ceil(2) })?;
                let palette = self.palette.as_ref().ok_or(TextureError::MissingPalette)?;
                let mut rgba = Vec::with_capacity(w * h * 4);
                for y in 0..h {
                    for x in 0..w {
                        let packed = mip.data[y * stride + x / 2];
                        let index = (packed >> ((x & 1) * 4)) & 15;
                        rgba.extend_from_slice(&palette.get(index as usize).copied().unwrap_or([0, 0, 0, 255]));
                    }
                }
                rgba
            }
            Format::Rgba8 => {
                check(w * h * 4)?;
                mip.data[..w * h * 4].chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect()
            }
            Format::Rgb8 => {
                check(w * h * 3)?;
                mip.data[..w * h * 3].chunks_exact(3).flat_map(|c| [c[2], c[1], c[0], 255]).collect()
            }
            Format::G16 => {
                check(w * h * 2)?;
                mip.data[..w * h * 2].chunks_exact(2).flat_map(|c| [c[1], c[1], c[1], 255]).collect()
            }
            Format::L8 => {
                check(w * h)?;
                mip.data[..w * h].iter().flat_map(|&l| [l, l, l, 255]).collect()
            }
            Format::Dxt1 => {
                check(blocks * 8)?;
                decode_blocks(&mip.data, w, h, 8, |b, out| decode_color(b, out, true))
            }
            Format::Dxt3 => {
                check(blocks * 16)?;
                decode_blocks(&mip.data, w, h, 16, |b, out| {
                    decode_color(&b[8..], out, false);
                    for (i, px) in out.iter_mut().enumerate() {
                        let nibble = (b[i / 2] >> ((i % 2) * 4)) & 0x0F;
                        px[3] = nibble * 17;
                    }
                })
            }
            Format::Dxt5 => {
                check(blocks * 16)?;
                decode_blocks(&mip.data, w, h, 16, |b, out| {
                    decode_color(&b[8..], out, false);
                    let (a0, a1) = (b[0] as u32, b[1] as u32);
                    let bits = b[2..8].iter().rev().fold(0u64, |acc, &x| acc << 8 | x as u64);
                    for (i, px) in out.iter_mut().enumerate() {
                        let code = ((bits >> (3 * i)) & 7) as u32;
                        px[3] = match code {
                            0 => a0,
                            1 => a1,
                            c if a0 > a1 => ((8 - c) * a0 + (c - 1) * a1) / 7,
                            6 => 0,
                            7 => 255,
                            c => ((6 - c) * a0 + (c - 1) * a1) / 5,
                        } as u8;
                    }
                })
            }
            other => return Err(TextureError::Unsupported(other)),
        };
        Ok((mip.width, mip.height, rgba))
    }
}

impl Texture {
    /// Data of a mip in a D3D destination: DXT kept, RGB8 as R5G6B5,
    /// other formats as BGRA8. Unwritten bytes stay as the caller left them.
    pub fn texture_data(&self, level: usize, pitch: usize, debug: bool, destination: &mut [u8]) -> Result<(), TextureError> {
        let mip = self.mips.get(level).ok_or(TextureError::NoMips)?;
        let (w, h) = (mip.width as usize, mip.height as usize);
        let block = match self.format { Format::Dxt1 => 8, Format::Dxt3 | Format::Dxt5 => 16, _ => 0 };
        let line = if block != 0 { (w.max(4) / 4) * block } else { w * if self.format == Format::Rgb8 { 2 } else { 4 } };
        let rows = if block != 0 { h.max(4) / 4 } else { h };
        if pitch < line { return Err(TextureError::BadSize { expected: line, got: pitch }); }
        if destination.len() < pitch * rows { return Err(TextureError::BadSize { expected: pitch * rows, got: destination.len() }); }
        if debug {
            // 24-bit x87 conversion, then the low byte is kept.
            let count = self.mips.len();
            let scaled = level as f32 * 255.0 / count as f32;
            let blue = (255.0f32 - scaled) as i32 as u8;
            let green = ((level as i32 - ((count / 2) / count) as i32).abs() as f32 * 255.0) as i32 as u8;
            let red = scaled as i32 as u8;
            if block != 0 {
                // The diagnostic blocks carry the colour in the INDICES, with zero endpoints.
                // Green is on five bits here and the terms are added, as in the asm.
                let color = ((red as u32 & 0xf8) << 8) + ((green as u32 & 0xf8) << 2) + (blue as u32 & 0xf8);
                for y in 0..rows {
                    for x in 0..line / block {
                        let target = &mut destination[y * pitch + x * block..][..block];
                        let offset = if block == 16 {
                            let alpha = if self.format == Format::Dxt3 { [0x77777777u32, 0x77777777] } else { [0xffffffffu32, 0x8081ffff] };
                            target[..4].copy_from_slice(&alpha[0].to_le_bytes());
                            target[4..8].copy_from_slice(&alpha[1].to_le_bytes());
                            8
                        } else { 0 };
                        target[offset..offset + 4].fill(0);
                        target[offset + 4..offset + 8].copy_from_slice(&color.to_le_bytes());
                    }
                }
            } else if self.format == Format::Rgb8 {
                let color = ((red as u16 >> 3) << 11) | ((green as u16 >> 2) << 5) | (blue as u16 >> 3);
                // Counts DWORDs but writes WORDs: half is kept.
                for target in destination[..pitch * h / 4 * 2].chunks_exact_mut(2) {
                    target.copy_from_slice(&color.to_le_bytes());
                }
            } else {
                for target in destination[..pitch * h / 4 * 4].chunks_exact_mut(4) {
                    target.copy_from_slice(&[blue, green, red, 0x80]);
                }
            }
            return Ok(());
        }
        if block != 0 {
            if mip.data.len() < line * rows { return Err(TextureError::BadSize { expected: line * rows, got: mip.data.len() }); }
            for y in 0..rows {
                destination[y * pitch..y * pitch + line].copy_from_slice(&mip.data[y * line..(y + 1) * line]);
            }
        } else {
            let (_, _, rgba) = self.decode(level)?;
            for y in 0..h {
                for x in 0..w {
                    let px = &rgba[(y * w + x) * 4..][..4];
                    if self.format == Format::Rgb8 {
                        let packed = ((px[0] as u16 >> 3) << 11) | ((px[1] as u16 >> 2) << 5) | (px[2] as u16 >> 3);
                        destination[y * pitch + x * 2..][..2].copy_from_slice(&packed.to_le_bytes());
                    } else {
                        destination[y * pitch + x * 4..][..4].copy_from_slice(&[px[2], px[1], px[0], px[3]]);
                    }
                }
            }
        }
        Ok(())
    }

    /// RGBA levels consumed by the Renderer, through the same GetTextureData transport.
    pub fn d3d_mips(&self) -> Vec<(u32, u32, Vec<u8>)> {
        let _ch = openxiii_package::loadings::start_value(openxiii_package::loadings::TEXTURE_DECODED);
        let mut out: Vec<(u32, u32, Vec<u8>)> = Vec::new();
        for level in 0..self.mips.len() {
            let mip = &self.mips[level];
            let (w, h) = (mip.width, mip.height);
            if let Some((pw, ph, _)) = out.last() {
                if w != (pw / 2).max(1) || h != (ph / 2).max(1) { break; }
            }
            let block = match self.format { Format::Dxt1 => 8, Format::Dxt3 | Format::Dxt5 => 16, _ => 0 };
            let pitch = if block != 0 { w.div_ceil(4) as usize * block } else { w as usize * if self.format == Format::Rgb8 { 2 } else { 4 } };
            let rows = if block != 0 { h.div_ceil(4) as usize } else { h as usize };
            let mut native = vec![0; pitch * rows];
            if self.texture_data(level, pitch, false, &mut native).is_err() { break; }
            let rgba = if block != 0 {
                let packed = Texture { format: self.format, mips: vec![Mip {width:w,height:h,data:native}], palette: None, trailing: Vec::new() };
                let Ok((_, _, rgba)) = packed.decode(0) else { break };
                rgba
            } else if self.format == Format::Rgb8 {
                // R5G6B5 read back by the graphics card as v/31 and v/63.
                native.chunks_exact(2).flat_map(|b| {
                    let v = u16::from_le_bytes([b[0], b[1]]) as u32;
                    let q = |v: u32, max: u32| ((v * 255 + max / 2) / max) as u8;
                    [q(v >> 11, 31), q((v >> 5) & 63, 63), q(v & 31, 31), 255]
                }).collect()
            } else {
                native.chunks_exact(4).flat_map(|b| [b[2], b[1], b[0], b[3]]).collect()
            };
            out.push((w, h, rgba));
        }
        out
    }
}

/// Palette: properties then TArray<FColor> (R, G, B, A).
pub fn read_palette(package: &Package, data: &[u8], index: usize) -> Result<Vec<[u8; 4]>, TextureError> {
    let object = package.object_data(data, index)?;
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let mut r = Reader::new(&data[..end], object.end);
    let count = r.compact()?.max(0) as usize;
    let bytes = r.take(count * 4)?;
    // Packages older than 66 force an opaque alpha.
    Ok(bytes.chunks_exact(4).map(|c| [c[0], c[1], c[2], if package.header.version < 0x42 { 255 } else { c[3] }]).collect())
}

fn rgb565(c: u16) -> [u32; 3] {
    let r = (c >> 11) as u32 & 31;
    let g = (c >> 5) as u32 & 63;
    let b = c as u32 & 31;
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

/// DXT colour block (8 bytes) to 16 RGBA pixels.
fn decode_color(b: &[u8], out: &mut [[u8; 4]; 16], dxt1: bool) {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mix = |a: u32, b: u32, wa: u32, wb: u32, d: u32| (a * wa + b * wb) / d;
    let mut colors = [[0u8; 4]; 4];
    colors[0] = [p0[0] as u8, p0[1] as u8, p0[2] as u8, 255];
    colors[1] = [p1[0] as u8, p1[1] as u8, p1[2] as u8, 255];
    if c0 > c1 || !dxt1 {
        for k in 0..3 {
            colors[2][k] = mix(p0[k], p1[k], 2, 1, 3) as u8;
            colors[3][k] = mix(p0[k], p1[k], 1, 2, 3) as u8;
        }
        colors[2][3] = 255;
        colors[3][3] = 255;
    } else {
        for k in 0..3 {
            colors[2][k] = mix(p0[k], p1[k], 1, 1, 2) as u8;
        }
        colors[2][3] = 255;
        colors[3] = [0, 0, 0, 0];
    }
    let indices = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    for (i, px) in out.iter_mut().enumerate() {
        *px = colors[((indices >> (2 * i)) & 3) as usize];
    }
}

fn decode_blocks(data: &[u8], w: usize, h: usize, block_size: usize, decode: impl Fn(&[u8], &mut [[u8; 4]; 16])) -> Vec<u8> {
    let mut rgba = vec![0u8; w * h * 4];
    let bw = w.div_ceil(4);
    for by in 0..h.div_ceil(4) {
        for bx in 0..bw {
            let offset = (by * bw + bx) * block_size;
            let mut px = [[0u8; 4]; 16];
            decode(&data[offset..offset + block_size], &mut px);
            for (i, p) in px.iter().enumerate() {
                let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                if x < w && y < h {
                    rgba[(y * w + x) * 4..][..4].copy_from_slice(p);
                }
            }
        }
    }
    rgba
}

/// Writes an RGBA image as PNG.
pub fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(rgba).map_err(std::io::Error::other)?;
    Ok(())
}

