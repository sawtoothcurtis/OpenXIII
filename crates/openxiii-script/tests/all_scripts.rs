//! Every script of the game packages reads without an unknown token. OpenXIII's bytecode reader rejects tokens
//! the VM does not know, including EX_DebugInfo (0x42): zero errors over all functions, states and class
//! scripts (replication blocks) proves that no game script contains one, so the branch of
//! UObject::CallFunction that skips debug info after EX_EndFunctionParms is unreachable.
//! Packages come from OXIII_GAME (System and Maps); without it the test says so and passes.

use openxiii_package::Package;
use openxiii_script::objects::{read_class, read_function, read_state};

#[test]
fn all_scripts_read() {
    let Ok(game_value) = openxiii_package::env::var("OXIII_GAME") else {
        eprintln!("all_scripts: OXIII_GAME not set, test skipped");
        return;
    };
    let (mut functions, mut states_list, mut classes, mut packages_list) = (0, 0, 0, 0);
    let mut errors_list = Vec::new();
    for folder in ["System", "Maps"] {
        let Ok(list_value) = std::fs::read_dir(std::path::Path::new(&game_value).join(folder)) else { continue };
        for input_value in list_value.flatten() {
            let path_buf = input_value.path();
            let ext = path_buf.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !["u", "unr"].contains(&ext.as_str()) {
                continue;
            }
            let data = std::fs::read(&path_buf).unwrap();
            let Ok(pkg) = Package::parse(&data) else { continue };
            packages_list += 1;
            for export in 0..pkg.exports.len() {
                if pkg.exports[export].serial_size == 0 {
                    continue;    // native class with no data in the package: nothing to read
                }
                let class_value = pkg.export_class(export).to_string();
                let name_value = path_buf.file_name().unwrap().to_string_lossy().to_string();
                match class_value.as_str() {
                    "Function" => {
                        functions += 1;
                        if let Err(e) = read_function(&pkg, &data, export) {
                            errors_list.push(format!("{name_value} export {export} Function: {e}"));
                        }
                    }
                    "State" => {
                        states_list += 1;
                        if let Err(e) = read_state(&pkg, &data, export) {
                            errors_list.push(format!("{name_value} export {export} State: {e}"));
                        }
                    }
                    "Class" => {
                        classes += 1;
                        if let Err(e) = read_class(&pkg, &data, export) {
                            errors_list.push(format!("{name_value} export {export} Class: {e}"));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    eprintln!("all_scripts: {packages_list} packages, {functions} functions, {states_list} states, {classes} classes");
    for e in errors_list.iter().take(10) {
        eprintln!("all_scripts: {e}");
    }
    assert!(packages_list > 0 && functions > 1000, "game packages not found");
    assert!(errors_list.is_empty(), "{} unreadable scripts", errors_list.len());
}
