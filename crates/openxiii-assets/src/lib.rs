//! Decoding of XIII's resources from package objects.

pub mod animation;
pub mod audio;
pub mod bsp;
pub mod cartoon;
pub mod character;
pub mod collision;
pub mod coords;
pub mod font;
pub mod hx;
pub mod level;
pub mod lighting;
pub mod lightmap;
/// Development diagnostics (OXIII_* variables), each read once.
mod diag;
pub mod material;
pub mod model;
pub mod particles;
pub mod printf;
pub mod props;
pub mod sine;
pub mod skeletal;
pub mod skeletal_collision;
pub mod static_mesh;
pub mod static_mesh_build;
pub mod stripify;
pub mod static_collision;
pub mod terrain;
pub mod terrain_postload;
pub mod terrain_sectors;
pub mod terrain_sector_init;
pub mod terrain_sector_buffer;
pub mod terrain_update;
pub mod texture;
/// Mods and improved builds: the mods' own textures and those made while the game runs.
#[cfg(feature = "runtime_textures")]
pub mod mod_textures;
/// Mods build only: the mods' own static meshes.
#[cfg(feature = "mods")]
pub mod mod_meshes;
#[cfg(feature = "mods")]
mod mod_assets;
#[cfg(feature = "mods")]
pub mod mod_skeletal;
/// Mods build only: first-person arms made from a game mesh (`[arms.<Name>]`).
#[cfg(feature = "mods")]
pub mod mod_arms;

/// A mod's static mesh for this name, and its material names (mods build only; never in the
/// faithful build).
pub(crate) fn mod_mesh(_name: &str) -> Option<(static_mesh::StaticMesh, Vec<String>)> {
    #[cfg(feature = "mods")]
    {
        if let Some(m) = mod_meshes::get(_name) {
            // As drawn: with its outline shell (mod_meshes::with_outline_shell). The collision of
            // the mods' meshes reads mod_meshes::get, without it.
            return Some(mod_meshes::with_outline_shell(m.0.clone(), m.1.clone()));
        }
    }
    None
}
pub mod ubi_adpcm;
pub mod ubi_stream;
pub mod world;
pub mod terrain_coords;

