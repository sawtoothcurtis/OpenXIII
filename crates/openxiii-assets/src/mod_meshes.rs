//! Mods build only (feature `mods`): the mods' own static meshes (glTF models), by mesh name.
//!
//! The props, actors and decorations of a level look up a static mesh here before the game's
//! packages: a mod's new mesh ("Mod.<Name>") or a game mesh a mod replaces. The table is set
//! once, when the game starts (docs/modding/README.md, "Your own 3D models").

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::static_mesh::{Section, StaticMesh};

/// A mesh, the material (texture) name of each of its sections, and what it collides with.
pub type ModMesh = Arc<(StaticMesh, Vec<String>, Collision)>;

/// What the actors drawn with a mod mesh collide with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Collision {
    /// The mesh's own triangles.
    #[default]
    Mesh,
    /// The box around the mesh's vertices.
    Box,
    /// Nothing.
    None,
}

static TABLE: OnceLock<HashMap<String, ModMesh>> = OnceLock::new();

/// Sets the mods' meshes, by name (once; a second call is ignored).
pub fn set(table: HashMap<String, ModMesh>) {
    let table = table.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect();
    let _ = TABLE.set(table);
}

/// The mod mesh drawn for static mesh `name`, if any.
pub fn get(name: &str) -> Option<ModMesh> {
    TABLE.get()?.get(&name.to_ascii_lowercase()).cloned()
}

/// UPrimitive+0x28..+0x30 bytes of a mod model: outline, cartoon shading, normals from the
/// position (never), outline thickness factor (`outline_width`, 1.0 by default) — the values most
/// of the game's meshes carry (`01 01 00 00, 00 00 80 3F`), so a mod model is drawn like a game
/// object.
pub fn cartoon_bytes(outline: bool, cartoon: bool, width: f32) -> [u8; 8] {
    let f = width.to_le_bytes();
    [outline as u8, cartoon as u8, 0, 0, f[0], f[1], f[2], f[3]]
}

/// Material name prefix of an outline shell section (`with_outline_shell`): "Mod.#outline:" then
/// the material of the section it outlines. `#` cannot appear in a mod's own names.
pub const OUTLINE_SHELL: &str = "Mod.#outline:";

/// The material outlined by an outline shell section of material `material`, if it is one.
pub fn outline_shell_of(material: &str) -> Option<&str> {
    material.strip_prefix(OUTLINE_SHELL)
}

/// The mesh and its materials as drawn: when the outline is on, one outline shell section is
/// added for each section: the same triangles on vertices shared by position, each with the
/// average of the normals of the faces around it. The game pushes the back faces along the
/// vertex normals to draw the black outline; a model made in a 3D editor usually has a vertex
/// per face at its hard edges (crates, guns), so its faces would come apart and the outline
/// would show as broken pieces. The shell is drawn only by the outline pass (the faces keep
/// their own normals and flat shading) and has no collision nor projector flag.
pub fn with_outline_shell(mut mesh: StaticMesh, mut materials: Vec<String>) -> (StaticMesh, Vec<String>) {
    if !mesh.outline() {
        return (mesh, materials);
    }
    let sections = mesh.sections.clone();
    materials.resize(materials.len().max(sections.len()), String::new());
    for (s, section) in sections.iter().enumerate() {
        let triangles = mesh.section_triangles(section);
        let mut slot: HashMap<[u32; 3], usize> = HashMap::new();
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut sums: Vec<Vec<[f32; 3]>> = Vec::new();
        let mut fallback: Vec<[f32; 3]> = Vec::new();
        let mut shell = Vec::with_capacity(triangles.len() * 3);
        for t in &triangles {
            let p = t.map(|i| mesh.positions[i as usize]);
            let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let mut n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            // The winding may go either way: the face normal follows the model's own normals.
            let given = t.iter().fold([0.0f32; 3], |a, &i| {
                let v = mesh.normals[i as usize];
                [a[0] + v[0], a[1] + v[1], a[2] + v[2]]
            });
            if n[0] * given[0] + n[1] * given[1] + n[2] * given[2] < 0.0 {
                n = n.map(|c| -c);
            }
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            for (k, &i) in t.iter().enumerate() {
                let key = p[k].map(f32::to_bits);
                let v = *slot.entry(key).or_insert_with(|| {
                    positions.push(p[k]);
                    sums.push(Vec::new());
                    fallback.push(mesh.normals[i as usize]);
                    positions.len() - 1
                });
                // Each face direction counts once around a corner (a flat face cut into several
                // triangles does not pull the normal towards it).
                if len > 1e-12 {
                    let u = n.map(|c| c / len);
                    if !sums[v].iter().any(|w: &[f32; 3]| w[0] * u[0] + w[1] * u[1] + w[2] * u[2] > 0.9999) {
                        sums[v].push(u);
                    }
                }
                shell.push(v);
            }
        }
        let base = mesh.positions.len();
        let first = mesh.indices.len();
        if positions.is_empty() || base + positions.len() > 65535 || first + shell.len() > 65535 {
            continue;
        }
        for (k, p) in positions.iter().enumerate() {
            let a = sums[k].iter().fold([0.0f32; 3], |a, u| [a[0] + u[0], a[1] + u[1], a[2] + u[2]]);
            let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
            mesh.positions.push(*p);
            mesh.normals.push(if len > 1e-12 { a.map(|c| c / len) } else { fallback[k] });
            for uv in mesh.uvs.iter_mut() {
                uv.push([0.0, 0.0]);
            }
        }
        mesh.indices.extend(shell.iter().map(|&v| (base + v) as u16));
        let count = (shell.len() / 3) as u16;
        mesh.sections.push(Section { is_strip: 0, first_index: first as u16, min_vertex: base as u16, max_vertex: (mesh.positions.len() - 1) as u16, triangles: count, primitives: count });
        mesh.materials.push(0);
        mesh.material_flags.push(0);
        let outlined = materials[s].clone();
        materials.push(format!("{OUTLINE_SHELL}{outlined}"));
    }
    (mesh, materials)
}

/// One section to build: positions, normals, first UV set, triangle indices, material name.
pub struct Part<'a> {
    pub positions: &'a [[f32; 3]],
    pub normals: &'a [[f32; 3]],
    pub uvs: &'a [[f32; 2]],
    pub indices: &'a [u32],
    pub material: String,
}

/// A StaticMesh made of `parts` (one section each). Err if it does not fit the game's 16-bit
/// indices.
pub fn build(parts: &[Part]) -> Result<(StaticMesh, Vec<String>), String> {
    let mut mesh = StaticMesh {
        bounds_min: [f32::MAX; 3],
        bounds_max: [f32::MIN; 3],
        sphere: [0.0; 4],
        unknown: [0; 8],
        sections: Vec::new(),
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: vec![Vec::new()],
        indices: Vec::new(),
        materials: Vec::new(),
        material_flags: Vec::new(),
    };
    let mut materials = Vec::new();
    for p in parts {
        let base = mesh.positions.len();
        let first = mesh.indices.len();
        if base + p.positions.len() > 65535 || first > 65535 || p.indices.len() / 3 > 65535 {
            return Err("too large: at most 65535 vertices and 65535 triangles in all".into());
        }
        if p.indices.iter().any(|&i| i as usize >= p.positions.len()) {
            return Err("an index points past the vertices".into());
        }
        for (i, v) in p.positions.iter().enumerate() {
            for k in 0..3 {
                mesh.bounds_min[k] = mesh.bounds_min[k].min(v[k]);
                mesh.bounds_max[k] = mesh.bounds_max[k].max(v[k]);
            }
            mesh.positions.push(*v);
            mesh.normals.push(p.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]));
            mesh.uvs[0].push(p.uvs.get(i).copied().unwrap_or([0.0, 0.0]));
        }
        mesh.indices.extend(p.indices.iter().map(|&i| (base + i as usize) as u16));
        let triangles = (p.indices.len() / 3) as u16;
        mesh.sections.push(Section { is_strip: 0, first_index: first as u16, min_vertex: base as u16, max_vertex: (mesh.positions.len().max(1) - 1) as u16, triangles, primitives: triangles });
        mesh.materials.push(0);
        // Bit 2: projectors (shadows, impacts) may draw on it, as on the game's meshes.
        mesh.material_flags.push(4);
        materials.push(p.material.clone());
    }
    if mesh.positions.is_empty() {
        return Err("no vertices".into());
    }
    let c = [0, 1, 2].map(|k| (mesh.bounds_min[k] + mesh.bounds_max[k]) / 2.0);
    let r = mesh.positions.iter().map(|p| ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt()).fold(0.0, f32::max);
    mesh.sphere = [c[0], c[1], c[2], r];
    Ok((mesh, materials))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cube with a vertex per face corner (24, as editors export hard edges).
    fn hard_cube() -> StaticMesh {
        let mut pos = Vec::new();
        let mut nor = Vec::new();
        let mut idx = Vec::new();
        for axis in 0..3 {
            for sign in [-1.0f32, 1.0] {
                let mut n = [0.0; 3];
                n[axis] = sign;
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let b = pos.len() as u32;
                for (a, c) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let mut p = [0.0; 3];
                    p[axis] = sign;
                    p[u] = a;
                    p[v] = c;
                    pos.push(p);
                    nor.push(n);
                }
                idx.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
            }
        }
        let part = Part { positions: &pos, normals: &nor, uvs: &[], indices: &idx, material: "Mod.Box.0".into() };
        let (mut mesh, _) = build(&[part]).unwrap();
        mesh.unknown = cartoon_bytes(true, true, 1.0);
        mesh
    }

    #[test]
    fn outline_shell_welds_hard_edges() {
        let mesh = hard_cube();
        let (shell, mats) = with_outline_shell(mesh.clone(), vec!["Mod.Box.0".into()]);
        assert_eq!(mats, ["Mod.Box.0", "Mod.#outline:Mod.Box.0"]);
        assert_eq!(outline_shell_of(&mats[1]), Some("Mod.Box.0"));
        assert_eq!(outline_shell_of(&mats[0]), None);
        // The model itself is unchanged; the shell has the 8 corners, each normal along the
        // diagonal (pointing out), and the same 12 triangles.
        assert_eq!(shell.sections[0].triangles, 12);
        assert_eq!(shell.positions.len(), 24 + 8);
        assert_eq!(shell.normals[..24], mesh.normals[..]);
        assert_eq!(shell.sections[1].triangles, 12);
        assert_eq!(shell.material_flags[1], 0);
        let d = 1.0 / 3.0f32.sqrt();
        for k in 24..32 {
            let (p, n) = (shell.positions[k], shell.normals[k]);
            for c in 0..3 {
                assert!((n[c] - p[c] * d).abs() < 1e-5, "{p:?} {n:?}");
            }
        }
        // Outline off: nothing added.
        let mut off = mesh.clone();
        off.unknown = cartoon_bytes(false, true, 1.0);
        let (same, m) = with_outline_shell(off, vec!["Mod.Box.0".into()]);
        assert_eq!((same.sections.len(), same.positions.len(), m.len()), (1, 24, 1));
    }

    #[test]
    fn build_and_lookup() {
        let pos = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]];
        let a = Part { positions: &pos[..3], normals: &[], uvs: &[], indices: &[0, 1, 2], material: "Mod.Paint".into() };
        let b = Part { positions: &pos[1..], normals: &[], uvs: &[], indices: &[0, 2, 1], material: String::new() };
        let (mesh, mats) = build(&[a, b]).unwrap();
        assert_eq!(mesh.positions.len(), 6);
        assert_eq!(mesh.sections.len(), 2);
        assert_eq!(mesh.section_triangles(&mesh.sections[1]), vec![[3, 5, 4]]);
        assert_eq!(mats, ["Mod.Paint", ""]);
        assert_eq!((mesh.bounds_min, mesh.bounds_max), ([0.0; 3], [10.0; 3]));
        assert_eq!(cartoon_bytes(true, true, 1.0), [1, 1, 0, 0, 0, 0, 0x80, 0x3F]);
        assert_eq!(cartoon_bytes(false, true, 2.0), [0, 1, 0, 0, 0, 0, 0, 0x40]);
        let mut t = HashMap::new();
        t.insert("Mod.Crate".to_string(), Arc::new((mesh, mats, Collision::Mesh)));
        set(t);
        assert!(get("mod.crate").is_some());
        assert!(build(&[Part { positions: &pos[..1], normals: &[], uvs: &[], indices: &[0, 1, 2], material: String::new() }]).is_err());
    }
}
