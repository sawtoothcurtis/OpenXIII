/// OXIII_BUILD_ID: what makes two builds play the same game (lockstep co-op meets only
/// identical ones): the source commit and whether the working tree had changes, the cargo
/// features that change the game, the pointer width and byte order,
/// and the profile. The renderer features (dx11, ...) and the operating system / processor
/// family are not part of it: the lockstep simulation uses portable math, so an Android and
/// a Windows build of the same sources play the same game.
pub fn configure() {
    // Cargo features that change the simulation (the others only change how it is drawn).
    const GAME_FEATURES: &[&str] = &["cheats", "improved", "mods"];
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&dir).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let commit = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "nogit".into());
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    let mut features: Vec<String> = std::env::vars().filter_map(|(k, _)| k.strip_prefix("CARGO_FEATURE_").map(|f| f.to_ascii_lowercase())).filter(|f| GAME_FEATURES.contains(&f.as_str())).collect();
    features.sort();
    let width = std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap_or_default();
    let endian = std::env::var("CARGO_CFG_TARGET_ENDIAN").unwrap_or_default();
    let profile = std::env::var("PROFILE").unwrap_or_default();
    let id = format!("{commit}{}|{}|release|{width}-{endian}|{profile}", if dirty { "+dirty" } else { "" }, features.join(","));
    println!("cargo:rustc-env=OXIII_BUILD_ID={id}");
    // Again when the commit or the sources change (the dirty flag).
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={}", std::path::Path::new(&dir).join(head).display());
    }
    if let Some(index) = git(&["rev-parse", "--git-path", "index"]) {
        println!("cargo:rerun-if-changed={}", std::path::Path::new(&dir).join(index).display());
    }
    println!("cargo:rerun-if-changed=../");
}
