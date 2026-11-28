//! Material dependencies of meshes and textures registered by mods.

/// A registered replacement takes precedence over a package export of the same name.
pub(crate) fn reachable(path: &str, materials: &mut Vec<String>, pending: &mut Vec<String>) -> bool {
    if let Some(material) = crate::mod_meshes::outline_shell_of(path) {
        pending.push(material.to_owned());
    } else if let Some(mesh) = crate::mod_meshes::get(path) {
        pending.extend(mesh.1.iter().filter(|s| !s.is_empty()).cloned());
    } else if let Some(mesh) = crate::mod_skeletal::get(path) {
        pending.extend(mesh.textures.iter().filter(|s| !s.is_empty()).cloned());
    } else if crate::mod_textures::get(path).is_some() {
        materials.push(path.to_owned());
    } else {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preloads_an_outline_material_and_its_registered_texture_without_a_package() {
        let name = "Mod.PreloadOutlineTest";
        crate::mod_textures::set_runtime(name, vec![(1, 1, vec![255; 4])]);
        let mut materials = Vec::new();
        let mut pending = Vec::new();
        let shell = format!("{}{name}", crate::mod_meshes::OUTLINE_SHELL);
        let shell_found = reachable(&shell, &mut materials, &mut pending);
        let texture_found = reachable(name, &mut materials, &mut pending);
        crate::mod_textures::remove_runtime(name);
        assert!(shell_found && texture_found);
        assert_eq!(pending, [name]);
        assert_eq!(materials, [name]);
    }
}
