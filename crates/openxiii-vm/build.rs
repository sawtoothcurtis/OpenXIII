//! VM build configuration and optional embedded game data.
use std::fmt::Write as _;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    embed_game_data();
}

/// Game data read by the game at run time (game_data.rs), embedded from this crate's game_data/
/// folder: the original's object order, the shader and texture prewarm lists, the Select level table.
const GAME_DATA: [&str; 7] = [
    "prewarm/lists.txt",
    "object_order/embedded_pass1.csv",
    "object_order/sounds_pass1.csv",
    "object_order/embedded_chain.csv",
    "object_order/sounds_chain.csv",
    "object_order/chain_maps.txt",
    "select_level/select_level.tsv",
];

fn embed_game_data() {
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("game_data");
    let mut code = String::from("/// Optional game data embedded at build time (build.rs): (path, contents).\nstatic EMBEDDED: &[(&str, &str)] = &[\n");
    for rel in GAME_DATA {
        let path = dir.join(rel);
        // Watching a missing path would rerun this script on every build: only existing ones.
        if path.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
            let full = path.canonicalize().unwrap();
            let _ = writeln!(code, "    ({rel:?}, include_str!({:?})),", full.display().to_string());
        }
    }
    code.push_str("];\n");
    std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("game_data.rs"), code).unwrap();
}
