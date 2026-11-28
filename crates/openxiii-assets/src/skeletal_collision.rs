//! Bone collision boxes of a skeletal mesh (USkeletalMesh + 0x1A8), used by
//! USkeletalMesh::LineCheck to find the bone hit by a shot (GetLastTraceBone).
//!
//! USkeletalMesh::Serialize, after what `skeletal::SkeletalMesh::read` reads
//! (up to the BoneInfluences, + 0x15C):
//! - fields +0x184 and +0x190: compact n × FName (compact index);
//! - field +0x19C: compact n × 12 × f32 (attachment FCoords);
//! - field +0x144: compact n × 9 × u16 (the rest of the 0x50-byte element is only
//!   serialized outside persistent packages);
//! - LicenseeVer < 0x17 only: a second array like + 0x144 (XIII: 58, absent);
//! - field +0x134: compact n × u16;
//! - field +0x1A8: compact n × [FBox (6 × f32 + IsValid byte), u16 bone].

use openxiii_package::{Package, Reader, Result};

/// 0x20-byte element: FBox at + 0x00 (Min, Max, IsValid) and bone index (ushort) at + 0x1C.
#[derive(Debug, Clone, Copy)]
pub struct BoneBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub valid: u8,
    pub bone: u16,
}

fn f32r(r: &mut Reader) -> Result<f32> {
    Ok(f32::from_bits(r.u32()?))
}

fn skip_u16s(r: &mut Reader) -> Result<()> {
    let n = r.compact()?;
    r.take(n.max(0) as usize * 2).map(|_| ())
}

/// Reader positioned at + 0x184 (right after the BoneInfluences).
fn skip_to_tags<'a>(package: &Package, data: &'a [u8], index: usize) -> Result<Reader<'a>> {
    let object = package.object_data(data, index)?;
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let mut r = Reader::new(&data[..end], object.end);
    // UPrimitive, ULodMesh: same fields as skeletal::SkeletalMesh::read.
    crate::skeletal::skip_mesh_prefix(&mut r, package.header.licensee)?;
    r.take(8)?; // Version, VertexCount
    let _ = crate::skeletal::read_legacy_vertices(&mut r)?;
    let _ = crate::skeletal::read_legacy_triangles(&mut r)?;
    for _ in 0..r.compact()? {
        r.compact()?; // Textures
    }
    r.take(12 + 12 + 12)?; // MeshScale, MeshOrigin, RotOrigin
    skip_u16s(&mut r)?; // CollapsePointThus
    skip_u16s(&mut r)?; // FaceLevel
    let n = r.compact()?;
    r.take(n.max(0) as usize * 8)?; // Faces
    skip_u16s(&mut r)?; // CollapseWedgeThus
    let n = r.compact()?;
    r.take(n.max(0) as usize * 10)?; // Wedges
    let n = r.compact()?;
    r.take(n.max(0) as usize * 8)?; // Materials
    r.take(3 * 4 + 4 + 4 + 4)?; // MeshScaleMax, LODHysteresis, LODStrength, LODMinVerts, LODMorph, LODZDisplace
                                // USkeletalMesh.
    let n = r.compact()?;
    r.take(n.max(0) as usize * 12)?; // Points (+ 0x110)
    for _ in 0..r.compact()? {
        r.compact()?; // RefSkeleton (+ 0x128): name, flags, quaternion, position, 4 × f32, parent
        r.take(4 + 16 + 12 + 16 + 4)?;
    }
    r.compact()?; // + 0x168 Animation
    r.i32()?; // + 0x140 SkeletalDepth
    for _ in 0..r.compact()? {
        skip_u16s(&mut r)?; // + 0x150 WeightIndices
        r.i32()?;
    }
    let n = r.compact()?;
    r.take(n.max(0) as usize * 4)?; // + 0x15C BoneInfluences
    Ok(r)
}

/// Mesh attachments: TagAliases (+ 0x184), TagNames (+ 0x190) and TagCoords (+ 0x19C,
/// FCoords origin, X, Y, Z), read by USkeletalMesh::SetAttachmentLocation.
#[derive(Debug, Clone, Default)]
pub struct AttachTags {
    /// Package name index (compact).
    pub aliases: Vec<i32>,
    pub names: Vec<i32>,
    pub coords: Vec<[f32; 12]>,
}

fn read_tags(r: &mut Reader) -> Result<AttachTags> {
    let aliases = (0..r.compact()?).map(|_| r.compact()).collect::<Result<Vec<_>>>()?;
    let names = (0..r.compact()?).map(|_| r.compact()).collect::<Result<Vec<_>>>()?;
    let coords = (0..r.compact()?)
        .map(|_| {
            let mut c = [0.0f32; 12];
            for v in c.iter_mut() {
                *v = f32r(r)?;
            }
            Ok(c)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(AttachTags { aliases, names, coords })
}

/// Attachments of a skeletal mesh.
pub fn read_attach_tags(package: &Package, data: &[u8], index: usize) -> Result<AttachTags> {
    let mut r = skip_to_tags(package, data, index)?;
    read_tags(&mut r)
}

/// Bone collision boxes (in array order, the order of the LineCheck test).
pub fn read_bone_boxes(package: &Package, data: &[u8], index: usize) -> Result<Vec<BoneBox>> {
    let mut r = skip_to_tags(package, data, index)?;
    read_tags(&mut r)?;
    let n = r.compact()?;
    r.take(n.max(0) as usize * 18)?; // + 0x144
    skip_u16s(&mut r)?; // + 0x134
    (0..r.compact()?)
        .map(|_| {
            let min = [f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?];
            let max = [f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?];
            let valid = r.u8()?;
            Ok(BoneBox { min, max, valid, bone: r.u16()? })
        })
        .collect()
}

/// UPrimitive::BoundingBox (+ 0x30), at the start of the native data of every UPrimitive
/// (mesh, StaticMesh): Min, Max, IsValid.
pub fn read_primitive_box(package: &Package, data: &[u8], index: usize) -> Result<([f32; 3], [f32; 3], u8)> {
    let object = package.object_data(data, index)?;
    let mut r = Reader::new(data, object.end);
    let min = [f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?];
    let max = [f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?];
    Ok((min, max, r.u8()?))
}

/// UPrimitive::BoundingSphere (+ 0x4C), after the box: centre and radius (FPlane).
pub fn read_primitive_sphere(package: &Package, data: &[u8], index: usize) -> Result<[f32; 4]> {
    let object = package.object_data(data, index)?;
    let mut r = Reader::new(data, object.end + 25);
    Ok([f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?])
}

