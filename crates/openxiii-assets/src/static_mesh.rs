//! XIII StaticMesh: vertices, UVs, triangles and per-material sections.
//!
//! Properties: `Materials` = native array (compact n, then per element u8 + compact
//! reference to the material). Native data, in order:
//! - FBox (6 × f32 + u8), sphere (4 × f32), 8 unknown bytes (often `01 01 00 00 00 00 80 3F`);
//! - sections (compact n × 14 bytes: i32 IsStrip, u16 FirstIndex, MinVertexIndex,
//!   MaxVertexIndex, NumTriangles, NumPrimitives);
//! - FBox; vertex stream (compact n × [position, normal] 6 × f32, i32 revision);
//! - UV streams (compact n streams; each compact n × 2 × f32, i32 coordinate, i32 revision);
//! - indices (compact n × u16, i32 revision), then wireframe and collision indices (not read).

use openxiii_package::{Package, Reader, Value};

#[derive(Debug, Clone)]
pub struct Section {
    pub is_strip: i32,
    pub first_index: u16,
    pub min_vertex: u16,
    pub max_vertex: u16,
    pub triangles: u16,
    pub primitives: u16,
}

#[derive(Debug, Clone)]
pub struct StaticMesh {
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// Bounding sphere (centre, radius).
    pub sphere: [f32; 4],
    /// UPrimitive+0x28..+0x2B then the float at +0x2C (UPrimitive::Serialize):
    /// outline, cartoon rendering, spherical normals (the shader uses the position as the normal),
    /// unused byte, outline thickness factor.
    pub unknown: [u8; 8],
    pub sections: Vec<Section>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub indices: Vec<u16>,
    /// Material of each section (package object reference, 0 = none).
    pub materials: Vec<i32>,
    /// Byte of each material (EnableCollision); loading also sets
    /// bit 2, which enables projectors, on all of them.
    pub material_flags: Vec<u8>,
}

fn vec3(r: &mut Reader) -> openxiii_package::Result<[f32; 3]> {
    Ok([f32::from_bits(r.u32()?), f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)])
}

// WORD TArray, same reader as the full StaticMesh path.
// TArray of vertices (position, normal); LicenseeVer < 34 element by element, which reads
// and discards two extra words below LicenseeVer 16.
fn read_position_normals(r: &mut Reader, licensee: u16) -> openxiii_package::Result<(Vec<[f32; 3]>, Vec<[f32; 3]>)> {
    let count = r.compact()?;
    let (mut positions, mut normals) = (Vec::new(), Vec::new());
    for _ in 0..count {
        positions.push(vec3(r)?);
        normals.push(vec3(r)?);
        if licensee < 16 {
            r.take(8)?;
        }
    }
    Ok((positions, normals))
}

fn read_index_words(r: &mut Reader) -> openxiii_package::Result<Vec<u16>> {
    (0..r.compact()?).map(|_| r.u16()).collect()
}

#[derive(Debug)]
pub struct MeshError(pub String);

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MeshError {}

impl From<openxiii_package::Error> for MeshError {
    fn from(e: openxiii_package::Error) -> Self {
        MeshError(e.to_string())
    }
}

/// Bytes UPrimitive+0x28..+0x30 of any mesh (StaticMesh, SkeletalMesh,
/// VertMesh): outline, cartoon, normals from position, then the thickness factor (f32).
/// UPrimitive::Serialize comes before the class's own data.
pub fn primitive_cartoon(package: &Package, data: &[u8], index: usize) -> Result<[u8; 8], MeshError> {
    let object = package.object_data(data, index)?;
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let mut r = Reader::new(&data[..end], object.end);
    r.take(25 + 16)?;
    Ok(r.take(8)?.try_into().unwrap())
}

impl StaticMesh {
    /// UPrimitive+0x28: outline drawn.
    pub fn outline(&self) -> bool {
        self.unknown[0] != 0
    }
    /// UPrimitive+0x29: cartoon rendering (lighting ramp).
    pub fn cartoon(&self) -> bool {
        self.unknown[1] != 0
    }
    /// UPrimitive+0x2A: normals taken from the position (shader).
    pub fn position_normals(&self) -> bool {
        self.unknown[2] != 0
    }
    /// UPrimitive+0x2C: outline thickness factor.
    pub fn outline_factor(&self) -> f32 {
        f32::from_le_bytes([self.unknown[4], self.unknown[5], self.unknown[6], self.unknown[7]])
    }

    /// The mesh as UStaticMesh::PostLoad leaves it: rebuilt by Build for
    /// versions 8 to 12 of +0x1BC.
    pub fn read(package: &Package, data: &[u8], index: usize) -> Result<Self, MeshError> {
        let mut mesh = Self::read_serialized(package, data, index)?;
        if let Ok((_, Some(raw))) = crate::static_collision::read_inner(package, data, index) {
            crate::static_mesh_build::apply_mesh(&mut mesh, &raw);
            mesh.validate()?;
        }
        Ok(mesh)
    }

    /// The mesh as UStaticMesh::Serialize reads it, before PostLoad.
    pub fn read_serialized(package: &Package, data: &[u8], index: usize) -> Result<Self, MeshError> {
        let object = package.object_data(data, index)?;
        let export = &package.exports[index];
        let end = export.serial_offset as usize + export.serial_size as usize;

        let mut materials = Vec::new();
        let mut material_flags = Vec::new();
        if let Some(Value::Raw(raw)) = object.get(package, "Materials") {
            let mut m = Reader::new(raw, 0);
            for _ in 0..m.compact()? {
                material_flags.push(m.u8()?);
                materials.push(m.compact()?);
            }
        }

        let mut r = Reader::new(&data[..end], object.end);
        let bounds_min = vec3(&mut r)?;
        let bounds_max = vec3(&mut r)?;
        r.u8()?;
        let sphere = [f32::from_bits(r.u32()?), f32::from_bits(r.u32()?), f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)];
        let unknown: [u8; 8] = r.take(8)?.try_into().unwrap();
        let sections = (0..r.compact()?)
            .map(|_| {
                Ok(Section { is_strip: r.i32()?, first_index: r.u16()?, min_vertex: r.u16()?, max_vertex: r.u16()?, triangles: r.u16()?, primitives: r.u16()? })
            })
            .collect::<openxiii_package::Result<Vec<_>>>()?;
        r.take(25)?; // second FBox
        let (positions, normals) = read_position_normals(&mut r, package.header.licensee)?;
        r.i32()?;
        let mut uvs = Vec::new();
        for _ in 0..r.compact()? {
            let n = r.compact()?;
            let stream = (0..n).map(|_| Ok([f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)])).collect::<openxiii_package::Result<Vec<_>>>()?;
            r.i32()?;
            r.i32()?;
            uvs.push(stream);
        }
        let indices = read_index_words(&mut r)?;
        r.i32()?;

        let mesh = StaticMesh { bounds_min, bounds_max, sphere, unknown, sections, positions, normals, uvs, indices, materials, material_flags };
        mesh.validate()?;
        Ok(mesh)
    }

    fn validate(&self) -> Result<(), MeshError> {
        let n = self.positions.len();
        if let Some(i) = self.indices.iter().find(|&&i| i as usize >= n) {
            return Err(MeshError(format!("index {i} outside the {n} vertices")));
        }
        if let Some(uv) = self.uvs.iter().find(|uv| uv.len() != n) {
            return Err(MeshError(format!("UV stream of {} entries for {n} vertices", uv.len())));
        }
        for s in &self.sections {
            let end = s.first_index as usize + s.triangles as usize * 3;
            if s.is_strip == 0 && end > self.indices.len() {
                return Err(MeshError(format!("section outside the indices ({end} > {})", self.indices.len())));
            }
        }
        Ok(())
    }

    /// Triangles (vertex indices) of a section.
    pub fn section_triangles(&self, s: &Section) -> Vec<[u16; 3]> {
        let start = s.first_index as usize;
        if s.is_strip != 0 {
            let strip = &self.indices[start..(start + s.triangles as usize + 2).min(self.indices.len())];
            strip
                .windows(3)
                .enumerate()
                .map(|(i, w)| if i % 2 == 0 { [w[0], w[1], w[2]] } else { [w[1], w[0], w[2]] })
                .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
                .collect()
        } else {
            self.indices[start..start + s.triangles as usize * 3].chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect()
        }
    }

    /// Wavefront OBJ export. Unreal is left-handed with Z up: we swap
    /// Y and Z (right-handed, Y up), which requires reversing the vertex order.
    pub fn to_obj(&self, material_names: &[String]) -> String {
        let mut s = String::new();
        for p in &self.positions {
            s += &format!("v {} {} {}\n", p[0], p[2], p[1]);
        }
        for n in &self.normals {
            s += &format!("vn {} {} {}\n", n[0], n[2], n[1]);
        }
        if let Some(uv) = self.uvs.first() {
            for t in uv {
                s += &format!("vt {} {}\n", t[0], 1.0 - t[1]);
            }
        }
        let has_uv = !self.uvs.is_empty();
        for (i, section) in self.sections.iter().enumerate() {
            let name = material_names.get(i).map_or("none", String::as_str);
            s += &format!("g section{i}\nusemtl {name}\n");
            for t in self.section_triangles(section) {
                let f = |i: u16| {
                    let i = i + 1;
                    if has_uv {
                        format!("{i}/{i}/{i}")
                    } else {
                        format!("{i}//{i}")
                    }
                };
                s += &format!("f {} {} {}\n", f(t[0]), f(t[2]), f(t[1]));
            }
        }
        s
    }
}

/// UStaticMeshInstance (UStaticMeshInstance::Serialize, licensee 58): properties,
/// colour stream (FRawColorStream: TArray<FColor> copied as is, B G R A) then
/// TArray<FStaticMeshLightInfo> (u16 light index, per-vertex visibility bits, integer).
#[derive(Debug, Clone)]
pub struct MeshInstance {
    /// Colour of each vertex as B, G, R, A.
    pub colors: Vec<[u8; 4]>,
    pub lights: Vec<crate::lighting::MeshLightInfo>,
}

impl MeshInstance {
    pub fn read(package: &Package, data: &[u8], index: usize) -> Result<Self, MeshError> {
        let object = package.object_data(data, index)?;
        let export = &package.exports[index];
        let end = export.serial_offset as usize + export.serial_size as usize;
        let mut r = Reader::new(&data[..end], object.end);
        let n = r.compact()?.max(0) as usize;
        let colors = r.take(n * 4)?.as_chunks::<4>().0.to_vec();
        let lights = (0..r.compact()?)
            .map(|_| {
                let light = r.u16()? as i16;
                let n = r.compact()?.max(0) as usize;
                let visibility = r.take(n)?.to_vec();
                let applied = r.i32()?;
                Ok(crate::lighting::MeshLightInfo { light, visibility, applied })
            })
            .collect::<openxiii_package::Result<Vec<_>>>()?;
        Ok(MeshInstance { colors, lights })
    }
}

