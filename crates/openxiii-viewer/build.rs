//! Embeds OpenXIII's icon (icon/openxiii.ico) and a common-controls manifest (resource 2) in oxiii-view.exe.
//!
//! No resource tool (rc.exe) or crate: we write the .res file ourselves, which
//! MSVC's linker accepts as is. One image per RT_ICON resource (1, 2, …) and the
//! RT_GROUP_ICON 1 group that lists them: Explorer and the taskbar take the
//! first group, and the window loads it by its number (main.rs, Icon::from_resource(1)).

use std::path::PathBuf;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_MANIFEST: u16 = 24;
const COMMON_CONTROLS_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

fn u16_value(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_value(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// one .res entry: 32-byte header (type and name by number), data aligned to 4
fn input_value(res: &mut Vec<u8>, kind: u16, name: u16, data: &[u8]) {
    res.extend_from_slice(&(data.len() as u32).to_le_bytes());
    res.extend_from_slice(&32u32.to_le_bytes());
    res.extend_from_slice(&[0xFF, 0xFF]);
    res.extend_from_slice(&kind.to_le_bytes());
    res.extend_from_slice(&[0xFF, 0xFF]);
    res.extend_from_slice(&name.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    res.extend_from_slice(&0x1010u16.to_le_bytes()); // MOVEABLE | DISCARDABLE
    res.extend_from_slice(&0x0409u16.to_le_bytes()); // English (United States)
    res.extend_from_slice(&0u32.to_le_bytes()); // Version
    res.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    res.extend_from_slice(data);
    while res.len() % 4 != 0 {
        res.push(0);
    }
}

mod build_id;

fn main() {
    build_id::configure();
    println!("cargo:rerun-if-changed=icon/openxiii.ico");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !(windows && msvc) {
        return;
    }
    let ico = std::fs::read("icon/openxiii.ico").expect("icon/openxiii.ico");
    let count = u16_value(&ico, 4) as usize;
    // empty header that starts every .res file
    let mut res = Vec::new();
    input_value(&mut res, 0, 0, &[]);
    res[20..24].copy_from_slice(&[0, 0, 0, 0]); // no flags, no language
    let mut group_value = Vec::new();
    group_value.extend_from_slice(&[0, 0, 1, 0]);
    group_value.extend_from_slice(&(count as u16).to_le_bytes());
    for i in 0..count {
        let e = &ico[6 + 16 * i..22 + 16 * i];
        let (size, offset) = (u32_value(e, 8) as usize, u32_value(e, 12) as usize);
        let id = i as u16 + 1;
        input_value(&mut res, RT_ICON, id, &ico[offset..offset + size]);
        // GRPICONDIRENTRY: the first 12 bytes of the .ico entry, then the number
        group_value.extend_from_slice(&e[..12]);
        group_value.extend_from_slice(&id.to_le_bytes());
    }
    input_value(&mut res, RT_GROUP_ICON, 1, &group_value);
    // Manifest 2 (not 1: the process itself runs without a manifest): common controls version 6,
    // activated only around the launcher's task dialog (desktop_shortcut.rs).
    input_value(&mut res, RT_MANIFEST, 2, COMMON_CONTROLS_MANIFEST.as_bytes());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("openxiii-icon.res");
    std::fs::write(&out, res).expect("writing the .res");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}
