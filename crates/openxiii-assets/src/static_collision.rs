//! Collision data of a StaticMesh (UStaticMesh::Serialize, licensee 58), after
//! what `static_mesh::StaticMesh::read` reads (up to the index buffer):
//! - wireframe indices (FRawIndexBuffer: compact n × u16, i32 revision);
//! - collision vertex stream (compact n × 3 × f32, i32 revision);
//! - collision triangles (FStaticMeshCollisionTriangle: 4 × u16,
//!   three vertices then the material);
//! - kDOP nodes (compact n then n × 12 raw bytes);
//! - the same triplet for simplified collision (licensee ≥ 0x27), then a lazy array
//!   (i32 end position, skipped) and an index buffer;
//! - object reference (licensee > 0x2A), then the flags UseSimpleBoxCollision (bit 0),
//!   UseSimpleLineCollision (bit 1) and bit 2 (licensee > 0x39).
//!
//! UStaticMesh::LineCheck uses the simplified collision when it exists and
//! (bit 1 and zero-extent trace) or (bit 0 and extent trace), except with TraceFlags 0x100.

use crate::static_mesh_build::RawTriangle;
use openxiii_package::{Package, Reader, Result, Value};

/// Collision tree node (FStaticMeshCollisionNode, 12 bytes): triangle, coplanar
/// node, back child, front child (0xFFFF: none), box quantised within the
/// parent's (5 bits per bound).
#[derive(Debug, Clone, Copy, Default)]
pub struct CollisionNode {
    pub triangle: u16,
    pub coplanar: u16,
    pub back: u16,
    pub front: u16,
    pub bounds: u32,
}

/// One collision set: vertices, triangles (indices, material) and tree.
#[derive(Debug, Clone, Default)]
pub struct CollisionSet {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u16; 4]>,
    pub nodes: Vec<CollisionNode>,
}

#[derive(Debug, Clone, Default)]
pub struct StaticCollision {
    pub full: CollisionSet,
    pub simple: CollisionSet,
    pub use_simple_box: bool,
    pub use_simple_line: bool,
    /// UStaticMesh+0x184 (short, licensee > 0x2D) after UStaticMesh::PostLoad:
    /// zero on load, it takes the number of RawTriangles. AActor::AttachProjector
    /// only attaches 10 − value/100 projectors.
    pub raw_triangles: i16,
    /// Materials read again at the end of the native part (licensee ≥ 0x38, after the lazy array of
    /// RawTriangles and the integer +0x1BC): object reference and FStaticMeshMaterial flags
    /// (+4: bit 0 = first byte of the file, bit 1 = second, which AActor::AttachProjector
    /// tests for each section: `test byte [Materials + section*8 + 4], 2`).
    /// Empty before licensee 0x38 (UStaticMesh::Serialize then sets bit 1 on all).
    pub native_materials: Vec<(i32, u8)>,
}

fn f32r(r: &mut Reader) -> Result<f32> {
    Ok(f32::from_bits(r.u32()?))
}

fn read_set(r: &mut Reader) -> Result<CollisionSet> {
    let vertices = (0..r.compact()?).map(|_| Ok([f32r(r)?, f32r(r)?, f32r(r)?])).collect::<Result<Vec<_>>>()?;
    r.i32()?;
    let triangles = (0..r.compact()?).map(|_| Ok([r.u16()?, r.u16()?, r.u16()?, r.u16()?])).collect::<Result<Vec<_>>>()?;
    // (licensee ≥ 0x22): count then the raw nodes.
    let nodes = (0..r.compact()?)
        .map(|_| Ok(CollisionNode { triangle: r.u16()?, coplanar: r.u16()?, back: r.u16()?, front: r.u16()?, bounds: r.u32()? }))
        .collect::<Result<Vec<_>>>()?;
    Ok(CollisionSet { vertices, triangles, nodes })
}

/// Full and simplified RawTriangles of a mesh with version +0x1BC between 8 and 12, which
/// UStaticMesh::PostLoad rebuilds through UStaticMesh::Build.
pub struct RawArrays {
    pub raw: Vec<RawTriangle>,
    pub simple: Vec<RawTriangle>,
    pub materials: Vec<(i32, u32)>,
}

/// FStaticMeshTriangle (archive version ≥ 0x10): three vertices, number of UV
/// sets then six floats per set, three colours (bytes 2, 1, 0, 3), material, smoothing.
fn read_triangle(r: &mut Reader) -> Result<RawTriangle> {
    let mut t = RawTriangle::default();
    for k in 0..3 {
        t.v[k] = [f32r(r)?, f32r(r)?, f32r(r)?];
    }
    t.nuv = r.i32()?;
    for s in 0..t.nuv.clamp(0, 8) as usize {
        for k in 0..3 {
            t.uv[k][s] = [f32r(r)?, f32r(r)?];
        }
    }
    r.take(12)?;
    t.mat = r.i32()?;
    t.smooth = r.u32()?;
    Ok(t)
}

/// A lazy array of FStaticMeshTriangle whose header (compact count) is at `pos`.
fn read_triangles(data: &[u8], pos: usize) -> Result<Vec<RawTriangle>> {
    let mut r = Reader::new(data, pos);
    (0..r.compact()?).map(|_| read_triangle(&mut r)).collect()
}

/// Collision of a StaticMesh of the package as UStaticMesh::Serialize reads it, before PostLoad.
pub fn read_serialized(package: &Package, data: &[u8], index: usize) -> Result<StaticCollision> {
    read_inner(package, data, index).map(|(c, _)| c)
}

/// Collision of a StaticMesh of the package, as PostLoad leaves it (Build for versions 8 to 12).
pub fn read(package: &Package, data: &[u8], index: usize) -> Result<StaticCollision> {
    read_inner(package, data, index).map(|(c, raw)| match raw {
        Some(raw) => crate::static_mesh_build::apply_collision(c, &raw),
        None => c,
    })
}

/// The collision as the file gives it and, if PostLoad rebuilds it, its RawTriangles.
pub fn read_inner(package: &Package, data: &[u8], index: usize) -> Result<(StaticCollision, Option<RawArrays>)> {
    let object = package.object_data(data, index)?;
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    // The number of materials does not affect the native data.
    let _ = object.get(package, "Materials").map(|v| matches!(v, Value::Raw(_)));
    let mut r = Reader::new(&data[..end], object.end);
    // UPrimitive: FBox, sphere, 8 bytes.
    r.take(25 + 16 + 8)?;
    // Sections (14 bytes).
    let n = r.compact()?;
    r.take(n.max(0) as usize * 14)?;
    r.take(25)?; // FBox
    // Vertices (position, normal) and revision.
    let n = r.compact()?;
    r.take(n.max(0) as usize * 24)?;
    r.i32()?;
    // UV streams.
    for _ in 0..r.compact()? {
        let n = r.compact()?;
        r.take(n.max(0) as usize * 8)?;
        r.i32()?;
        r.i32()?;
    }
    // Indices, then wireframe indices.
    for _ in 0..2 {
        let n = r.compact()?;
        r.take(n.max(0) as usize * 2)?;
        r.i32()?;
    }
    let full = read_set(&mut r)?;
    let simple = read_set(&mut r)?;
    // (Ver ≥ 0x3E): end position of the lazy array.
    let skip = r.i32()?;
    let simple_at = r.pos;
    r.pos = skip as usize;
    let n = r.compact()?;
    r.take(n.max(0) as usize * 2)?;
    r.i32()?;
    r.compact()?; // object reference
    // First boolean: bit 0 of +0x188 (zero-extent traces), second: bit 1 (extent traces and
    // PointCheck), see UStaticMesh::LineCheck and PointCheck.
    let use_simple_line = r.u32()? & 1 != 0;
    let use_simple_box = r.u32()? & 1 != 0;
    // Bit 2 (licensee > 0x39), short +0x184 (licensee > 0x2D), two i32 +0x17C/+0x180
    // (licensee > 0x30), then RawTriangles (TLazyArray: i32 end position, compact n).
    // Bit 2 (u32) only exists in packages with licensee ≥ 0x3A: XIIICamp.utx
    // (licensee 57) does not have it.
    let licensee = package.header.licensee;
    let end_value = |r: &mut Reader| -> Result<(i16, i32, usize)> {
        if licensee >= 0x3A {
            r.u32()?;
        }
        let value_read = r.u16()? as i16;
        r.i32()?;
        r.i32()?;
        let lazy_end = r.i32()?;
        let at = r.pos;
        let n = r.compact()?;
        Ok((if value_read == 0 { n as i16 } else { value_read }, lazy_end, at))
    };
    let (raw_triangles, lazy_end, raw_at) = end_value(&mut r).unwrap_or((0, 0, 0));
    // After the RawTriangles: integer +0x1BC, then Materials (compact n, per
    // element the object reference then two bytes).
    let mut version = 13;
    let native_materials = (|| -> Result<Vec<(i32, u8)>> {
        r.pos = usize::try_from(lazy_end).map_err(|_| openxiii_package::Error::BadIndex(lazy_end))?;
        version = r.i32()?;
        (0..r.compact()?).map(|_| Ok((r.compact()?, (r.u8()? & 1) | (r.u8()? & 1) << 1))).collect()
    })()
    .unwrap_or_default();
    let collision = StaticCollision { full, simple, use_simple_box, use_simple_line, raw_triangles, native_materials };
    // PostLoad rebuilds meshes of versions 8 to 12 (below that, or −1: RawTriangles rebuilt from the sections).
    let raw = if (8..13).contains(&version) && raw_at != 0 {
        (|| -> Result<RawArrays> {
            let materials = if collision.native_materials.is_empty() {
                match object.get(package, "Materials") {
                    Some(Value::Raw(raw)) => {
                        let mut m = Reader::new(raw, 0);
                        let n = m.compact()?;
                        (0..n).map(|_| Ok({ let f = m.u8()?; (m.compact()?, (f as u32 & 1) | 2) })).collect::<Result<Vec<_>>>()?
                    }
                    _ => Vec::new(),
                }
            } else {
                collision.native_materials.iter().map(|&(r, f)| (r, f as u32)).collect()
            };
            Ok(RawArrays { raw: read_triangles(&data[..end], raw_at)?, simple: read_triangles(&data[..end], simple_at)?, materials })
        })()
        .ok()
    } else {
        None
    };
    Ok((collision, raw))
}
