//! Mods build only (feature `mods`): the mods' own textures, by texture name.
//!
//! The game window and the level's surfaces look up a texture here before the game's packages:
//! a mod's new texture ("Mod.<Name>") or a game texture a mod replaces. The table is set once,
//! when the game starts (docs/modding/README.md, "Your own textures").

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Mip levels, largest first: (width, height, RGBA bytes).
pub type Mips = Vec<(u32, u32, Vec<u8>)>;

/// Mip levels of an RGBA image, down to 1 × 1 (each texel the mean of 2 × 2 of the level above).
pub fn mips(w: u32, h: u32, rgba: Vec<u8>) -> Mips {
    let mut out: Mips = vec![(w, h, rgba)];
    loop {
        let (pw, ph, prev) = out.last().unwrap();
        if *pw <= 1 && *ph <= 1 {
            break;
        }
        let (pw, ph) = (*pw, *ph);
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

static TABLE: OnceLock<HashMap<String, Arc<Mips>>> = OnceLock::new();

/// Sets the mods' textures, by name (once; a second call is ignored).
pub fn set(table: HashMap<String, Arc<Mips>>) {
    let table = table.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect();
    let _ = TABLE.set(table);
}

/// The mod texture drawn for texture `name`, if any (the mods' textures, then the textures made
/// while the game runs).
pub fn get(name: &str) -> Option<Arc<Mips>> {
    let key = name.to_ascii_lowercase();
    if let Some(m) = TABLE.get().and_then(|t| t.get(&key)) {
        return Some(m.clone());
    }
    RUNTIME.get()?.read().ok()?.get(&key).cloned()
}

/// Textures made while the game runs (the improved build's main menu boxes), by name.
static RUNTIME: OnceLock<std::sync::RwLock<HashMap<String, Arc<Mips>>>> = OnceLock::new();

/// Adds or replaces a texture made while the game runs ("Mod.<Name>"). A texture of the mods
/// with the same name keeps priority.
pub fn set_runtime(name: &str, mips: Mips) {
    let table = RUNTIME.get_or_init(Default::default);
    if let Ok(mut t) = table.write() {
        t.insert(name.to_ascii_lowercase(), Arc::new(mips));
    }
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
}

/// Removes a texture made while the game runs: the game's own texture of that name is drawn again.
pub fn remove_runtime(name: &str) {
    if let Some(Ok(mut f)) = FINE.get().map(|f| f.write()) {
        f.remove(&name.to_ascii_lowercase());
    }
    let Some(table) = RUNTIME.get() else { return };
    let removed = table.write().is_ok_and(|mut t| t.remove(&name.to_ascii_lowercase()).is_some());
    if removed {
        GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
    }
}

/// Number of textures made or replaced while the game runs so far: a renderer that sees it
/// change looks again at the textures it uploaded (or did not find) under those names.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// See GENERATION.
pub fn runtime_generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

/// True if `name` is a texture made while the game runs (never worth remembering between runs:
/// it does not exist yet when the next run starts).
pub fn is_runtime(name: &str) -> bool {
    let key = name.to_ascii_lowercase();
    RUNTIME.get().and_then(|t| t.read().ok()).is_some_and(|t| t.contains_key(&key))
}

/// Textures made while the game runs from one large master picture (the improved menus'
/// characters and onomatopoeias): the renderer reads them with a sharper filter than the
/// bilinear one (bicubic, from the mip level just larger than the screen size), so the same
/// picture stays sharp at any screen size and nothing is made again when the size changes.
static FINE: OnceLock<std::sync::RwLock<std::collections::HashSet<String>>> = OnceLock::new();

/// Adds or replaces a runtime texture (`set_runtime`) read with the sharper filter.
pub fn set_runtime_fine(name: &str, mips: Mips) {
    if let Ok(mut f) = FINE.get_or_init(Default::default).write() {
        f.insert(name.to_ascii_lowercase());
    }
    set_runtime(name, mips);
}

/// True if `name` is read with the sharper filter (see `set_runtime_fine`).
pub fn is_fine(name: &str) -> bool {
    FINE.get().and_then(|t| t.read().ok()).is_some_and(|t| !t.is_empty() && t.contains(&name.to_ascii_lowercase()))
}

/// Marks a texture the renderer makes itself (not in the runtime table) as read with the
/// sharper filter.
pub fn mark_fine(name: &str) {
    if let Ok(mut f) = FINE.get_or_init(Default::default).write() {
        f.insert(name.to_ascii_lowercase());
    }
}

static CLAMPED: OnceLock<std::collections::HashSet<String>> = OnceLock::new();

/// The mods' new textures read at their edge outside the image instead of repeating
/// (`clamp = true`: UClampMode and VClampMode at TC_Clamp), by name (once).
pub fn set_clamped(names: std::collections::HashSet<String>) {
    let _ = CLAMPED.set(names.into_iter().map(|n| n.to_ascii_lowercase()).collect());
}

/// True if mod texture `name` does not repeat (`clamp = true`).
pub fn clamped(name: &str) -> bool {
    CLAMPED.get().is_some_and(|s| s.contains(&name.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "mods")]
    use std::path::PathBuf;
    use std::sync::Arc;

    /// The table of these tests (set once per process, the same for every test).
    fn table() {
        let mut t = std::collections::HashMap::new();
        t.insert("Mod.Poster".to_string(), Arc::new(vec![(1, 1, vec![1, 2, 3, 4])]));
        t.insert("XIIIPlage.PLspot".to_string(), Arc::new(vec![(2, 2, vec![9; 16]), (1, 1, vec![9; 4])]));
        super::set(t);
    }

    #[test]
    fn lookup_ignores_case() {
        table();
        assert_eq!(super::get("mod.POSTER").unwrap()[0].2, vec![1, 2, 3, 4]);
        assert!(super::get("XIIIPlage.PLsky02").is_none());
    }

    /// The level's surfaces (material_image) get the mod's image of a replaced texture
    /// (OXIII_GAME = original game folder; skipped when it is not set).
    // Mods build only: the surfaces and materials read the mods' table under the "mods" feature.
    #[cfg(feature = "mods")]
    #[test]
    fn surfaces_use_replaced_textures() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(PathBuf::from).filter(|g| openxiii_package::system_dir(g).exists()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        table();
        let mut library = openxiii_package::library::Library::new(&game).unwrap();
        let images = crate::world::material_image(&mut library, "XIIIPlage.PLspot").expect("image");
        assert_eq!((images[0].width, images[0].height), (2, 2));
        assert_eq!(images[0].rgba, vec![9; 16]);
        // A texture no mod replaces comes from the game.
        let other = crate::world::material_image(&mut library, "XIIIPlage.PLsky02").expect("game image");
        assert!(other[0].width > 2);
    }

    /// A new mod texture (in no package) is a Texture for the materials, with its image's size
    /// and alpha, so props and characters draw it (OXIII_GAME; skipped when it is not set).
    // Mods build only: the surfaces and materials read the mods' table under the "mods" feature.
    #[cfg(feature = "mods")]
    #[test]
    fn new_textures_are_textures_for_the_materials() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(PathBuf::from).filter(|g| openxiii_package::system_dir(g).exists()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        table();
        let mut library = openxiii_package::library::Library::new(&game).unwrap();
        let mut materials = crate::material::Materials::new();
        let info = materials.texture_info(&mut library, "Mod.Poster").expect("a texture");
        assert_eq!((info.usize, info.vsize), (1, 1));
        // Its one pixel has alpha 4: drawn with its alpha.
        assert!(info.alpha_texture && !info.masked);
        assert!(materials.material_is_a(&mut library, "Mod.Poster", "BitmapMaterial"));
        // A replaced game texture keeps the game's object.
        let spot = materials.texture_info(&mut library, "XIIIPlage.PLspot").expect("game texture");
        assert!(spot.usize > 2);
        assert!(materials.texture_info(&mut library, "Mod.NoSuchTexture").is_none());
    }
}
