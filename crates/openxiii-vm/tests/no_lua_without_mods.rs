//! The faithful build has no Lua: the Lua crates are dependencies of the `mods` feature only
//! (of openxiii-vm and of the game window). Read from cargo's own dependency tree.

use std::process::Command;

fn tree(package: &str, features: &[&str]) -> String {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(env!("CARGO_MANIFEST_DIR")).args(["tree", "--offline", "-e", "normal", "--prefix", "none", "-p", package]);
    if !features.is_empty() {
        cmd.args(["--features", &features.join(",")]);
    }
    let out = cmd.output().expect("cargo tree");
    assert!(out.status.success(), "cargo tree -p {package}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn has_lua(tree: &str) -> bool {
    tree.lines().any(|l| l.starts_with("mlua") || l.starts_with("lua-src") || l.starts_with("mlua-sys"))
}

#[test]
fn faithful_build_has_no_lua() {
    for package in ["openxiii-vm", "openxiii-viewer"] {
        assert!(!has_lua(&tree(package, &[])), "{package} without features has no Lua");
    }
    // Negative control: with the mods feature, Lua is there.
    assert!(has_lua(&tree("openxiii-vm", &["mods"])), "openxiii-vm with mods has Lua");
    assert!(has_lua(&tree("openxiii-viewer", &["mods"])), "openxiii-viewer with mods has Lua");
}
