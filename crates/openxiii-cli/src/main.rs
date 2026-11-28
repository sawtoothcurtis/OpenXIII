use openxiii_package::{ObjectRef, Package};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Like println!, but exits without error if the output is closed (e.g. `| head`).
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let tag = crate::OUT_TAG.with(|t| t.borrow().clone());
        if writeln!(std::io::stdout(), "{tag}{}", format_args!($($arg)*)).is_err() {
            std::process::exit(0);
        }
    }};
}

thread_local! {
    /// Prefix of this thread's output lines (`oxiii loop`: "[host]", "[client]").
    static OUT_TAG: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static SEG_T0: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

mod improved;
mod launcher;
mod server;

/// USAGE, followed by the development commands when they are built.
fn usage() -> String {
    USAGE.to_string()
}

const USAGE: &str = "\
Usage:
  oxiii info <package>      Header and number of entries
  oxiii names <package>     Name table
  oxiii imports <package>   Import table
  oxiii exports <package>   Export table
  oxiii props <package> <export>  Properties of an export (name, path or index)
  oxiii scan <folder>       Reads every package of a folder (recursive)
  oxiii propscan <folder>   Reads the properties of every object (statistics)
  oxiii textures <package|folder> <output>  Exports the textures as PNG
  oxiii hx <bank.hxc>       Lists the sounds of an HX bank
  oxiii hxscan <folder>     Statistics on every HX bank
  oxiii sounds <bank.hxc|folder> <output>  Exports the sounds as WAV (PCM and Ubi ADPCM)
  oxiii uaxscan <game>      Links the packages' Sound objects to the HX banks
  oxiii meshes <package|folder> [output]  Reads the StaticMeshes (and exports them as OBJ)
  oxiii skeletal <package|folder> [output]  Reads the SkeletalMeshes (reference pose as OBJ)
  oxiii anims <package|folder>  Reads the MeshAnimations (statistics)
  oxiii pose <game> <Package.Mesh> <sequence> <frame> <output.obj>  Exports an animated pose
  oxiii level <game> <map.unr> <output.obj>  Assembles the StaticMeshes placed in a map
  oxiii scriptscan <folder>  Reads the bytecode of every Function/State/Class
  oxiii disasm <package> <export>  Disassembles a Function/State/Class
  oxiii classinfo <game> <class[,class…]>  Package, flags, ini, parent class and number of inherited properties
  oxiii funcinfo <game> <class[,class…]>  Exec functions declared by the classes and their parameters
  oxiii proptree <game> <class[,class…]>  Property tree of the classes
  oxiii defaults-offsets <package.u>  For each Class: export, name, start and end of the defaultproperties
  oxiii objscan <folder>    Reads every script object (functions, states, classes, properties)
  oxiii sound <game> <map> <Package.Sound> <output.wav>  Decodes a Sound object (banks of the map and Fix)
  oxiii hxrun <game> <map> <handler> [seconds] [parameters…]  Runs an HX sound program handler and traces the voices
  oxiii probes <folder>     Event masks (ProbeMask/IgnoreMask) of the classes and states, with their functions
  oxiii decompile <game> <package.u> <output>  Decompiles the classes into .uc files
  oxiii vmtest <game>       Runs Epic's UnrealScript test suite (Engine.TestInfo)
  oxiii layout <game> <class>  Memory position (C++) of each property
  oxiii run <game> <class> <function> [arguments…]  Runs a script function
  oxiii server <game> <URL> [seconds] [-port=N]  Dedicated server like \"UCC server\" (real time)
  oxiii launcher <game> [mode map players=N ff points=N time=N name=X port=N]  Dedicated server launcher (ServerLauncher.exe)
  oxiii mods-reference <game> <folder>  Writes the modding reference pages from the game's own files (--features mods)";

const PACKAGE_EXTENSIONS: &[&str] = &["u", "unr", "utx", "usx", "uax", "ukx", "uvx"];

/// Original game folder for "ucc": OXIII_GAME, otherwise the folder above ucc.exe if it contains
/// System\Engine.u (ucc.exe placed in the game), otherwise GameDir from %APPDATA%\OpenXIII\OpenXIII.ini (remembered by XIII.exe).
fn ucc_game_dir(exe_dir: &Path) -> Option<std::path::PathBuf> {
    if let Ok(j) = openxiii_package::env::var("OXIII_GAME") {
        return Some(j.into());
    }
    if let Some(up) = exe_dir.parent().filter(|u| openxiii_package::system_dir(u).join("Engine.u").is_file()) {
        return Some(up.to_path_buf());
    }
    let ini = std::path::PathBuf::from(openxiii_package::env::var_os("APPDATA")?).join("OpenXIII").join("OpenXIII.ini");
    let text = std::fs::read_to_string(ini).ok()?;
    text.lines().find_map(|l| l.trim().strip_prefix("GameDir=").map(|v| v.trim().into()))
}

/// Executable renamed "ucc.exe": replaces UCC.exe for the original ServerLauncher.exe, which runs
/// `ucc server <URL> -port=N` from its current folder (ShellExecuteA). Only "server" exists;
/// the log goes to UCC.log next to ucc.exe, like the original (System\UCC.log).
fn ucc_main(exe_dir: &Path, args: &[String]) -> ExitCode {
    if !args.first().is_some_and(|c| c.eq_ignore_ascii_case("server")) {
        eprintln!("OpenXIII's ucc: only the command \"server <URL> [-port=N]\" exists (dedicated server).");
        return ExitCode::from(2);
    }
    let Some(game) = ucc_game_dir(exe_dir) else {
        eprintln!("Original game folder not found: start OpenXIII's XIII.exe once first.");
        return ExitCode::from(1);
    };
    let mut line: Vec<String> = args.to_vec();
    let log = exe_dir.join("UCC.log");
    let has_log = line.iter().any(|a| a.to_ascii_uppercase().contains("LOG="));
    if !has_log && !log.to_string_lossy().contains(' ') {
        line.push(format!("-ABSLOG={}", log.display()));
    }
    println!("Original game: {}", game.display());
    #[cfg(feature = "mods")]
    openxiii_vm::mod_setup::start_mods(&line, Some(&game), &mut |l| println!("{l}"));
    match server::server(&game, &line) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(exe) = std::env::current_exe() {
        if exe.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("ucc")) {
            return ucc_main(exe.parent().unwrap_or(Path::new(".")), &args);
        }
    }
    let (Some(command), Some(path)) = (args.first(), args.get(1)) else {
        eprintln!("{}", usage());
        return ExitCode::from(2);
    };
    let path = Path::new(path);
    // Mods build: the commands that run the game (server, simulated player, loop) load the mods
    // first, as the game window does (--no-mods or OXIII_NO_MODS=1 to run without).
    #[cfg(feature = "mods")]
    if matches!(command.as_str(), "server" | "simulate" | "loop" | "campaign") {
        openxiii_vm::mod_setup::start_mods(&args, Some(path), &mut |line| println!("{line}"));
    }
    let result = match command.as_str() {
        "scan" => scan(path),
        "propscan" => propscan(path),
        "hx" => hx_list(path),
        "hxscan" => hx_scan(path),
        "uaxscan" => uax_scan(path),
        "scriptscan" => script_scan(path),
        "objscan" => obj_scan(path),
        "probes" => probes(path),
        "sound" => match (args.get(2), args.get(3), args.get(4)) {
            (Some(map), Some(sound), Some(out)) => sound_to_wav(path, map, sound, Path::new(out)),
            _ => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "hxrun" => match args.get(2) {
            Some(map) => match args.get(3) {
                Some(handler) => hx_run(path, map, handler, args.get(4).and_then(|s| s.parse().ok()).unwrap_or(3.0), &args[5.min(args.len())..]),
                None => {
                    eprintln!("{}", usage());
                    return ExitCode::from(2);
                }
            },
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "vmtest" => vm_test(path),
        // Mods build: oxiii mods-reference <game> <folder>, writes weapons.md and characters.md
        // (the modding reference of the weapon, ammo and character classes) into <folder>.
        #[cfg(feature = "mods")]
        "mods-reference" => match args.get(2) {
            Some(dir) => (|| -> Result<(), String> {
                let mut vm = openxiii_vm::Vm::new();
                vm.ipdrv.offline = true;
                vm.load_dir(&openxiii_package::system_dir(&path)).map_err(|e| e.to_string())?;
                let dir = Path::new(dir);
                let mut pages = vec![("weapons.md".into(), vm.weapons_reference()?), ("characters.md".into(), vm.characters_reference()?), ("animations.md".into(), vm.animations_reference()?)];
                pages.extend(vm.content_references()?);
                pages.push(("maps.md".into(), openxiii_vm::Vm::maps_reference(path)?));
                pages.push(("character-models.md".into(), openxiii_vm::Vm::character_models_reference(path)?));
                pages.push(("dialogues.md".into(), vm.dialogues_reference()?));
                for (name, text) in pages {
                    let out: std::path::PathBuf = dir.join(name);
                    if let Some(parent) = out.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                    }
                    std::fs::write(&out, text).map_err(|e| format!("{}: {e}", out.display()))?;
                }
                println!("written: {} (weapons, characters, animations, maps, dialogues, sounds and content references)", dir.display());
                Ok(())
            })(),
            None => {
                eprintln!("usage: oxiii mods-reference <game folder> <output folder>");
                return ExitCode::from(2);
            }
        },
        // oxiii proptree <game> <class>: property tree for tests (layout.rs, prop_tree).
        "proptree" => match args.get(2) {
            Some(class) => (|| -> Result<(), String> {
                let mut vm = openxiii_vm::Vm::new();
                vm.ipdrv.offline = true;
                vm.load_dir(&openxiii_package::system_dir(&path)).map_err(|e| e.to_string())?;
                // Several classes separated by commas: a single load.
                for c in class.split(',') {
                    print!("{}", vm.prop_tree(c).ok_or(format!("unknown class: {c}"))?);
                }
                Ok(())
            })(),
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        // oxiii classinfo <game> <classes>: "C name package flags ini parent inherited" per class (for LoadConfig tests).
        "classinfo" => match args.get(2) {
            Some(classes) => (|| -> Result<(), String> {
                let mut vm = openxiii_vm::Vm::new();
                vm.ipdrv.offline = true;
                vm.load_dir(&openxiii_package::system_dir(&path)).map_err(|e| e.to_string())?;
                for c in classes.split(',') {
                    print!("{}", vm.class_info(c).ok_or(format!("unknown class: {c}"))?);
                }
                Ok(())
            })(),
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        // oxiii funcinfo <game> <classes> [all]: exec functions (or all of them with "all") and parameters (for ScriptConsoleExec and parms_size tests).
        "funcinfo" => match args.get(2) {
            Some(classes) => (|| -> Result<(), String> {
                let mut vm = openxiii_vm::Vm::new();
                vm.ipdrv.offline = true;
                vm.load_dir(&openxiii_package::system_dir(&path)).map_err(|e| e.to_string())?;
                let everything = args.get(3).is_some_and(|a| a == "all");
                for c in classes.split(',') {
                    let info = if everything { vm.func_info_all(c) } else { vm.func_info(c) };
                    print!("{}", info.ok_or(format!("unknown class: {c}"))?);
                }
                Ok(())
            })(),
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "layout" => match args.get(2) {
            Some(class) if class == "--json" => layouts_json(path),
            Some(class) => layout(path, class),
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        // oxiii server <game> <URL> [seconds] [-port=N]: dedicated server like "UCC server"
        // (server.rs), in real time, with neither a window nor a local player.
        // oxiii launcher <game> [mode map settings…]: like ServerLauncher.exe (launcher.rs), then oxiii server.
        "launcher" => launcher::launcher(path, &args[2..]),
        "server" => match args.get(2) {
            Some(_) => server::server(path, &args[2..]),
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "run" => match (args.get(2), args.get(3)) {
            (Some(class), Some(function)) => vm_run(path, class, function, &args[4..]),
            _ => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "decompile" => match (args.get(2), args.get(3)) {
            (Some(pkg), Some(out)) => decompile(path, Path::new(pkg), Path::new(out)),
            _ => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "defaults-offsets" => defaults_offsets(path),
        "disasm" => match args.get(2) {
            Some(target) => disasm(path, target),
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "meshes" => meshes(path, args.get(2).map(Path::new)),
        "skeletal" => skeletal(path, args.get(2).map(Path::new)),
        "anims" => anims(path),
        "pose" => match (args.get(2), args.get(3), args.get(4), args.get(5)) {
            (Some(mesh), Some(seq), Some(frame), Some(out)) => pose(path, mesh, seq, frame.parse().unwrap_or(0.0), Path::new(out)),
            _ => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "bspscan" => bsp_scan(path),
        "ray" => match (args.get(2), args.get(3), args.get(4)) {
            (Some(map), Some(from), Some(dir)) => ray(path, map, from, dir),
            _ => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "level" => match (args.get(2), args.get(3)) {
            (Some(map), Some(out)) => level(path, Path::new(map), Path::new(out)),
            _ => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "sounds" => match args.get(2) {
            Some(out) => sounds(path, Path::new(out)),
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "textures" => match args.get(2) {
            Some(out) => textures(path, Path::new(out)),
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "props" => match args.get(2) {
            Some(target) => props(path, target),
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
        "info" | "names" | "imports" | "exports" => load(path).map(|p| match command.as_str() {
            "info" => info(&p),
            "names" => names(&p),
            "imports" => imports(&p),
            _ => exports(&p),
        }),
        _ => match improved::command(command, path, &args[2..]) {
            Some(result) => result,
            None => {
                eprintln!("{}", usage());
                return ExitCode::from(2);
            }
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn load(path: &Path) -> Result<Package, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Package::parse(&data).map_err(|e| format!("{}: {e}", path.display()))
}

fn info(p: &Package) {
    let h = &p.header;
    out!("version      {} (licensee {})", h.version, h.licensee);
    out!("flags        0x{:08X}", h.flags);
    out!("names        {} @ 0x{:X}", h.name_count, h.name_offset);
    out!("imports      {} @ 0x{:X}", h.import_count, h.import_offset);
    out!("exports      {} @ 0x{:X}", h.export_count, h.export_offset);
    let guid: String = h.guid.iter().map(|b| format!("{b:02X}")).collect();
    out!("guid         {guid}");
    for (i, g) in h.generations.iter().enumerate() {
        out!("generation {i} {} exports, {} names", g.export_count, g.name_count);
    }
    let mut classes: BTreeMap<&str, usize> = BTreeMap::new();
    for i in 0..p.exports.len() {
        *classes.entry(p.export_class(i)).or_default() += 1;
    }
    out!("export classes:");
    for (class, count) in classes {
        out!("  {count:6} {class}");
    }
}

fn names(p: &Package) {
    for (i, n) in p.names.iter().enumerate() {
        out!("{i:6} 0x{:08X} {}", n.flags, n.name);
    }
}

fn imports(p: &Package) {
    for i in 0..p.imports.len() {
        out!("{:6} {:28} {}", -(i as i64) - 1, p.import_class(i), p.object_path(ObjectRef::Import(i)));
    }
}

fn exports(p: &Package) {
    out!("{:>6} {:24} {:10} {:>9} {:>10}  path", "index", "class", "flags", "size", "offset");
    for (i, e) in p.exports.iter().enumerate() {
        out!("{:6} {:24} 0x{:08X} {:9} 0x{:08X}  {}", i + 1, p.export_class(i), e.flags, e.serial_size, e.serial_offset, p.object_path(ObjectRef::Export(i)));
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| PACKAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())) {
            out.push(path);
        }
    }
    Ok(())
}

fn scan(dir: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect(dir, &mut files).map_err(|e| format!("{}: {e}", dir.display()))?;
    files.sort();
    let (mut ok, mut failed) = (0, 0);
    let (mut names, mut imports, mut exports) = (0, 0, 0);
    let mut versions: BTreeMap<(u16, u16), usize> = BTreeMap::new();
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    for file in &files {
        match load(file) {
            Ok(p) => {
                ok += 1;
                names += p.names.len();
                imports += p.imports.len();
                exports += p.exports.len();
                *versions.entry((p.header.version, p.header.licensee)).or_default() += 1;
                for i in 0..p.exports.len() {
                    *classes.entry(p.export_class(i).to_string()).or_default() += 1;
                }
                out!(
                    "ok  {:6} names {:6} imports {:6} exports  {}",
                    p.names.len(),
                    p.imports.len(),
                    p.exports.len(),
                    file.strip_prefix(dir).unwrap_or(file).display()
                );
            }
            Err(e) => {
                failed += 1;
                out!("ERR {e}");
            }
        }
    }
    out!("
{ok} packages read, {failed} with errors; {names} names, {imports} imports, {exports} exports");
    for ((v, l), n) in versions {
        out!("version {v} licensee {l}: {n} packages");
    }
    let mut classes: Vec<_> = classes.into_iter().collect();
    classes.sort_by(|a, b| b.1.cmp(&a.1));
    out!("most frequent export classes:");
    for (class, count) in classes.iter().take(25) {
        out!("  {count:7} {class}");
    }
    if failed > 0 {
        Err(format!("{failed} unreadable packages"))
    } else {
        Ok(())
    }
}

fn props(path: &Path, target: &str) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let p = Package::parse(&data).map_err(|e| e.to_string())?;
    let indices: Vec<usize> = match target.parse::<usize>() {
        Ok(n) if (1..=p.exports.len()).contains(&n) => vec![n - 1],
        _ => (0..p.exports.len())
            .filter(|&i| p.object_path(ObjectRef::Export(i)).eq_ignore_ascii_case(target) || p.name(p.exports[i].object_name).eq_ignore_ascii_case(target))
            .collect(),
    };
    if indices.is_empty() {
        return Err(format!("export not found: {target}"));
    }
    for i in indices {
        let e = &p.exports[i];
        out!("{} ({}) export {}, {} bytes @ 0x{:X}", p.object_path(ObjectRef::Export(i)), p.export_class(i), i + 1, e.serial_size, e.serial_offset);
        if p.export_class(i) == "Class" {
            out!("  (Class object: specific serialization, not read)");
            continue;
        }
        let o = p.object_data(&data, i).map_err(|e| e.to_string())?;
        if let Some(s) = &o.state {
            out!("  state: node {} statenode {} probemask 0x{:016X} latent {} offset {:?}", s.node, s.state_node, s.probe_mask, s.latent_action, s.offset);
        }
        for prop in &o.properties {
            let index = if prop.array_index > 0 { format!("[{}]", prop.array_index) } else { String::new() };
            let st = prop.struct_name.map(|n| format!(" ({})", p.name(n))).unwrap_or_default();
            out!("  {}{index}{st} = {}", p.name(prop.name), p.display_value(&prop.value));
        }
        let rest = e.serial_offset as usize + e.serial_size as usize - o.end;
        out!("  … {rest} bytes of class-specific data");
    }
    Ok(())
}

fn propscan(dir: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect(dir, &mut files).map_err(|e| format!("{}: {e}", dir.display()))?;
    files.sort();
    let (mut objects, mut failed, mut properties) = (0usize, 0usize, 0usize);
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: Vec<String> = Vec::new();
    // Per class: (objects, objects with native data after the properties, native bytes).
    let mut native: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for file in &files {
        let data = std::fs::read(file).map_err(|e| e.to_string())?;
        let p = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        for i in 0..p.exports.len() {
            if p.export_class(i) == "Class" || p.exports[i].serial_size == 0 {
                continue;
            }
            objects += 1;
            match p.object_data(&data, i) {
                Ok(o) => {
                    properties += o.properties.len();
                    let e = &p.exports[i];
                    let rest = e.serial_offset as usize + e.serial_size as usize - o.end;
                    let entry = native.entry(p.export_class(i).to_string()).or_default();
                    entry.0 += 1;
                    entry.1 += (rest > 0) as usize;
                    entry.2 += rest;
                }
                Err(e) => {
                    failed += 1;
                    *failures.entry(p.export_class(i).to_string()).or_default() += 1;
                    if examples.len() < 20 {
                        examples.push(format!("{} export {} ({}): {e}", file.display(), i + 1, p.export_class(i)));
                    }
                }
            }
        }
    }
    out!("{} packages, {objects} objects, {properties} properties, {failed} objects with errors", files.len());
    let mut native: Vec<_> = native.into_iter().filter(|(_, v)| v.1 > 0).collect();
    native.sort_by(|a, b| b.1 .2.cmp(&a.1 .2));
    out!("classes with native data after the properties (objects concerned / total, bytes):");
    for (class, (total, with, bytes)) in native.iter().take(40) {
        out!("  {class:28} {with:6} / {total:6}  {bytes:12}");
    }
    for (class, n) in &failures {
        out!("  {n:7} failures {class}");
    }
    for e in examples {
        out!("  {e}");
    }
    if failed > 0 {
        Err(format!("{failed} unreadable objects"))
    } else {
        Ok(())
    }
}

fn textures(input: &Path, out: &Path) -> Result<(), String> {
    use openxiii_assets::texture::{write_png, Texture};
    let mut files = Vec::new();
    if input.is_dir() {
        collect(input, &mut files).map_err(|e| e.to_string())?;
    } else {
        files.push(input.to_path_buf());
    }
    files.sort();
    let (mut written, mut failed) = (0usize, 0usize);
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut trailing: BTreeMap<usize, usize> = BTreeMap::new();
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    for file in &files {
        let data = std::fs::read(file).map_err(|e| e.to_string())?;
        let p = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("package");
        for i in 0..p.exports.len() {
            if p.export_class(i) != "Texture" {
                continue;
            }
            let result = Texture::read(&p, &data, i).and_then(|t| {
                *formats.entry(format!("{:?}", t.format)).or_default() += 1;
                *trailing.entry(t.trailing.len()).or_default() += 1;
                t.decode(0)
            });
            match result {
                Ok((w, h, rgba)) => {
                    let name = p.object_path(ObjectRef::Export(i)).replace(['/', '\\', ':'], "_");
                    let dir = out.join(stem);
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    write_png(&dir.join(format!("{name}.png")), w, h, &rgba).map_err(|e| e.to_string())?;
                    written += 1;
                }
                Err(e) => {
                    failed += 1;
                    *errors.entry(e.to_string()).or_default() += 1;
                }
            }
        }
    }
    out!("{written} textures exported, {failed} failed, in {}", out.display());
    out!("formats: {formats:?}");
    out!("bytes after the mips: {trailing:?}");
    for (e, n) in errors {
        out!("  {n:6} {e}");
    }
    Ok(())
}

fn hx_files(input: &Path) -> Result<Vec<PathBuf>, String> {
    if !input.is_dir() {
        return Ok(vec![input.to_path_buf()]);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(input)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("hxc")))
        .collect();
    files.sort();
    Ok(files)
}

fn load_hx(path: &Path) -> Result<openxiii_assets::hx::HxBank, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    openxiii_assets::hx::HxBank::parse(&data).map_err(|e| format!("{}: {e}", path.display()))
}

fn hx_list(path: &Path) -> Result<(), String> {
    use openxiii_assets::hx::Location;
    let bank = load_hx(path)?;
    let mut classes: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &bank.entries {
        *classes.entry(e.class.as_str()).or_default() += 1;
    }
    out!("index type {}, {} entries: {classes:?}", bank.index_type, bank.entries.len());
    for w in &bank.waves {
        let place = match &w.location {
            Location::Internal { offset, size } => format!("internal 0x{offset:X} {size}"),
            Location::External { resource, offset, size } => format!("{resource} 0x{offset:X} {size}"),
        };
        out!(
            "{} {:12} {}ch {:5}Hz mode {:2}  {:32} {place}",
            w.cuuid,
            format!("{:?}", w.codec),
            w.channels,
            w.sample_rate,
            w.mode,
            w.name.as_deref().unwrap_or("?")
        );
        if !w.markers.is_empty() {
            let shown: Vec<String> = w.markers.iter().take(12).map(|(p, n)| format!("{n}@{p}")).collect();
            out!("    {} markers: {}{}", w.markers.len(), shown.join(" "), if w.markers.len() > 12 { " …" } else { "" });
        }
    }
    for (class, e) in &bank.failures {
        out!("FAILED {class}: {e}");
    }
    Ok(())
}

fn hx_scan(dir: &Path) -> Result<(), String> {
    use openxiii_assets::hx::Location;
    let (mut banks, mut waves, mut failures, mut unnamed) = (0, 0, 0, 0);
    let mut codecs: BTreeMap<String, usize> = BTreeMap::new();
    let mut resources: BTreeMap<String, usize> = BTreeMap::new();
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    for file in hx_files(dir)? {
        let bank = load_hx(&file)?;
        banks += 1;
        waves += bank.waves.len();
        failures += bank.failures.len();
        for e in &bank.entries {
            *classes.entry(e.class.clone()).or_default() += 1;
        }
        for w in &bank.waves {
            unnamed += w.name.is_none() as usize;
            *codecs.entry(format!("{:?} {}ch {}Hz mode {}", w.codec, w.channels, w.sample_rate, w.mode)).or_default() += 1;
            if let Location::External { resource, .. } = &w.location {
                *resources.entry(resource.clone()).or_default() += 1;
            }
        }
        for (class, e) in bank.failures.iter().take(3) {
            out!("FAILED {} {class}: {e}", file.display());
        }
    }
    out!("{banks} banks, {waves} sounds, {failures} failures, {unnamed} without a name");
    out!("classes: {classes:?}");
    for (k, n) in codecs {
        out!("  {n:6} {k}");
    }
    out!("external resources: {}", resources.len());
    for (k, n) in resources.iter().take(10) {
        out!("  {n:6} {k}");
    }
    Ok(())
}

fn sounds(input: &Path, out: &Path) -> Result<(), String> {
    use openxiii_assets::hx::{write_wav, Codec, Location};
    let (mut written, mut skipped) = (0, 0);
    let mut skipped_codecs: BTreeMap<String, usize> = BTreeMap::new();
    for file in hx_files(input)? {
        let data = std::fs::read(&file).map_err(|e| e.to_string())?;
        let bank = load_hx(&file)?;
        let dir = file.parent().unwrap_or(Path::new("."));
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("banque");
        let target = out.join(stem);
        std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        let mut external: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut used = std::collections::HashSet::new();
        for w in &bank.waves {
            if !matches!(w.codec, Codec::Pcm16 | Codec::UbiAdpcm) {
                skipped += 1;
                *skipped_codecs.entry(format!("{:?}", w.codec)).or_default() += 1;
                continue;
            }
            let samples: &[u8] = match &w.location {
                Location::Internal { offset, size } => data.get(*offset as usize..(*offset + *size) as usize).ok_or("internal sound outside the file")?,
                Location::External { resource, offset, size } => {
                    if !external.contains_key(resource) {
                        let name = Path::new(resource).file_name().unwrap_or_default();
                        let bytes = std::fs::read(dir.join(name)).map_err(|e| format!("{resource}: {e}"))?;
                        external.insert(resource.clone(), bytes);
                    }
                    external[resource].get(*offset as usize..(*offset + *size) as usize).ok_or("external sound outside the file")?
                }
            };
            let base = w.name.clone().unwrap_or_else(|| w.cuuid.to_string());
            let base = match &w.language {
                Some(lang) => format!("{base}_{lang}"),
                None => base,
            };
            let mut name = base.replace(['/', '\\', ':'], "_");
            // Several waves (languages, variants) can share a name.
            if !used.insert(name.to_lowercase()) {
                name = format!("{name}_{}", w.cuuid);
            }
            let pcm: Vec<u8> = if w.codec == Codec::UbiAdpcm {
                match openxiii_assets::ubi_adpcm::decode(samples) {
                    Ok((_, pcm)) => pcm.iter().flat_map(|s| s.to_le_bytes()).collect(),
                    Err(e) => {
                        skipped += 1;
                        *skipped_codecs.entry(format!("UbiAdpcm: {e}")).or_default() += 1;
                        continue;
                    }
                }
            } else {
                samples.to_vec()
            };
            write_wav(&target.join(format!("{name}.wav")), w.channels, w.sample_rate, &pcm).map_err(|e| e.to_string())?;
            written += 1;
        }
    }
    out!("{written} sounds exported, {skipped} skipped (codec): {skipped_codecs:?}");
    Ok(())
}

fn uax_scan(game: &Path) -> Result<(), String> {
    use openxiii_assets::hx::{sound_reference, Cuuid};
    let mut resources: std::collections::HashMap<Cuuid, (String, String)> = Default::default();
    let mut entry_classes: std::collections::HashMap<Cuuid, String> = Default::default();
    let mut by_class: BTreeMap<String, usize> = BTreeMap::new();
    for file in hx_files(&game.join("Sounds").join("PC"))? {
        let bank = load_hx(&file)?;
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string();
        for w in &bank.waves {
            if let (Some(r), Some(n)) = (w.resource, &w.name) {
                resources.entry(r).or_insert_with(|| (stem.clone(), n.clone()));
            }
        }
        for e in &bank.entries {
            entry_classes.entry(e.cuuid).or_insert_with(|| e.class.clone());
        }
    }
    let mut files = Vec::new();
    collect(game, &mut files).map_err(|e| e.to_string())?;
    files.sort();
    let (mut sounds, mut found, mut same_name) = (0, 0, 0);
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    let mut missing = Vec::new();
    for file in &files {
        let data = std::fs::read(file).map_err(|e| e.to_string())?;
        let p = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        for i in 0..p.exports.len() {
            if p.export_class(i) != "Sound" {
                continue;
            }
            sounds += 1;
            let (file_type, id) = sound_reference(&p, &data, i).map_err(|e| e.to_string())?;
            *types.entry(file_type).or_default() += 1;
            let object = p.name(p.exports[i].object_name);
            let class = id.and_then(|id| entry_classes.get(&id)).cloned().unwrap_or_else(|| "missing".into());
            *by_class.entry(class).or_default() += 1;
            match id.and_then(|id| resources.get(&id)) {
                Some((_, name)) => {
                    found += 1;
                    same_name += name.eq_ignore_ascii_case(object) as usize;
                }
                None if !entry_classes.contains_key(&id.unwrap_or(Cuuid(0, 0))) => {
                    missing.push(format!("{} {}", file.display(), p.object_path(ObjectRef::Export(i))))
                }
                None => {}
            }
        }
    }
    out!("{sounds} Sound objects, {found} linked to a named sound ({same_name} with the same name), {} without an HX entry", missing.len());
    out!("types: {types:?}");
    out!("HX classes referenced: {by_class:?}");
    for m in missing.iter().take(15) {
        out!("  not found: {m}");
    }
    Ok(())
}

fn meshes(input: &Path, out: Option<&Path>) -> Result<(), String> {
    use openxiii_assets::static_mesh::StaticMesh;
    let mut files = Vec::new();
    if input.is_dir() {
        collect(input, &mut files).map_err(|e| e.to_string())?;
    } else {
        files.push(input.to_path_buf());
    }
    files.sort();
    let (mut ok, mut failed, mut vertices, mut triangles, mut strips) = (0, 0, 0, 0, 0);
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors: Vec<String> = Vec::new();
    for file in &files {
        let data = std::fs::read(file).map_err(|e| e.to_string())?;
        let p = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("package");
        for i in 0..p.exports.len() {
            if p.export_class(i) != "StaticMesh" {
                continue;
            }
            match StaticMesh::read(&p, &data, i) {
                Ok(mesh) => {
                    ok += 1;
                    vertices += mesh.positions.len();
                    triangles += mesh.sections.iter().map(|s| s.triangles as usize).sum::<usize>();
                    strips += mesh.sections.iter().filter(|s| s.is_strip != 0).count();
                    let key: String = mesh.unknown.iter().map(|b| format!("{b:02x}")).collect();
                    *unknown.entry(key).or_default() += 1;
                    if let Some(out) = out {
                        let dir = out.join(stem);
                        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                        let name = p.object_path(ObjectRef::Export(i)).replace(['/', '\\', ':'], "_");
                        let materials: Vec<String> = mesh.materials.iter().map(|&m| if m == 0 { "none".into() } else { p.object_path(m.into()) }).collect();
                        let mut obj = format!("mtllib {name}.mtl\n");
                        obj += &mesh.to_obj(&materials);
                        let mtl: String = materials.iter().map(|m| format!("newmtl {m}\nKd 1 1 1\nmap_Kd {m}.png\n")).collect();
                        std::fs::write(dir.join(format!("{name}.obj")), obj).map_err(|e| e.to_string())?;
                        std::fs::write(dir.join(format!("{name}.mtl")), mtl).map_err(|e| e.to_string())?;
                    }
                }
                Err(e) => {
                    failed += 1;
                    if errors.len() < 20 {
                        errors.push(format!("{} {}: {e}", file.display(), p.object_path(ObjectRef::Export(i))));
                    }
                }
            }
        }
    }
    out!("{ok} StaticMesh read, {failed} failed; {vertices} vertices, {triangles} triangles, {strips} strip sections");
    let mut unknown: Vec<_> = unknown.into_iter().collect();
    unknown.sort_by(|a, b| b.1.cmp(&a.1));
    out!("8 unknown bytes (most frequent values):");
    for (k, n) in unknown.iter().take(8) {
        out!("  {n:6} {k}");
    }
    for e in errors {
        out!("  {e}");
    }
    Ok(())
}

/// Ray cast into a map's blocking geometry: first contact and material.
/// Reads all the maps' UModels (BspModel) and checks the leaves' hulls.
fn bsp_scan(game: &Path) -> Result<(), String> {
    use openxiii_assets::bsp::BspModel;
    let mut maps: Vec<PathBuf> = std::fs::read_dir(game.join("Maps"))
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "unr"))
        .collect();
    maps.sort();
    let (mut ok, mut bad) = (0usize, 0usize);
    for map in maps {
        let data = std::fs::read(&map).map_err(|e| e.to_string())?;
        let package = Package::parse(&data).map_err(|e| e.to_string())?;
        let (mut n, mut nodes, mut hulls, mut errors) = (0usize, 0usize, 0usize, Vec::new());
        // PF_Mirrored surfaces (0x8000000, mirror branch) of all the map's models.
        let mut mirrors = 0usize;
        // Brush models (not the map's BSP) whose Bounds, LeafHulls or Leaves are not empty: what ABrush::PostLoad
        // empties in game for volumes; zero everywhere = no effect on volume collisions.
        let map_value = openxiii_assets::model::level_model(&package, &data);
        let mut brushes_full = 0usize;
        // Brushes (Brush, volumes) rotated in the file: ABrush::PostLoad resets their Rotation to zero in game.
        let mut brushes_turned = Vec::new();
        for i in 0..package.exports.len() {
            let c = package.export_class(i);
            if c != "Brush" && !c.ends_with("Volume") {
                continue;
            }
            if let Ok(o) = package.object_data(&data, i) {
                // bHidden = False saved: ABrush::PostLoad sets it back to true for volumes (and PF_Invisible).
                if let Some(v) = o.get(&package, "bHidden") {
                    brushes_turned.push(format!("{} bHidden {v:?}", package.object_path(ObjectRef::Export(i))));
                }
                if let Some(v) = o.get(&package, "Rotation") {
                    let s = format!("{v:?}");
                    if s.chars().any(|ch| ch.is_ascii_digit() && ch != '0') {
                        brushes_turned.push(format!("{} {s}", package.object_path(ObjectRef::Export(i))));
                    }
                }
            }
        }
        for x in &brushes_turned {
            out!("  rotated or visible brush: {x}");
        }
        for i in 0..package.exports.len() {
            if package.export_class(i) != "Model" || package.exports[i].serial_size == 0 {
                continue;
            }
            match BspModel::read(&package, &data, i) {
                Ok(m) => {
                    n += 1;
                    nodes += m.nodes.len();
                    mirrors += m.surfs.iter().filter(|s| s.poly_flags & 0x800_0000 != 0).count();
                    if Some(i) != map_value && !(m.leaf_hulls.is_empty() && m.bounds.is_empty() && m.leaf_zones.is_empty()) {
                        brushes_full += 1;
                    }
                    // Each iCollisionBound designates a list terminated by −1 followed by 6 floats.
                    for node in &m.nodes {
                        if node.collision_bound >= 0 {
                            let s = node.collision_bound as usize;
                            match m.leaf_hulls[s..].iter().position(|&x| x == -1) {
                                Some(k) if s + k + 7 <= m.leaf_hulls.len() => hulls += 1,
                                _ => errors.push(format!("{}: invalid envelope {s}", package.object_path(ObjectRef::Export(i)))),
                            }
                        }
                    }
                }
                Err(e) => errors.push(format!("{}: {e}", package.object_path(ObjectRef::Export(i)))),
            }
        }
        ok += n;
        bad += errors.len();
        out!("{}: {n} models, {nodes} nodes, {hulls} hulls, {mirrors} mirror surfaces, {brushes_full} brushes with Bounds/LeafHulls/Leaves, {} errors", map.file_stem().unwrap_or_default().to_string_lossy(), errors.len());
        if let Some(l) = openxiii_assets::model::level_model(&package, &data) {
            if let Ok(m) = BspModel::read(&package, &data, l) {
                out!(
                    "  BSP of map {}: {} nodes, {} surfaces, {} zones, {} hulls (i32), RootOutside {}",
                    package.object_path(ObjectRef::Export(l)),
                    m.nodes.len(),
                    m.surfs.len(),
                    m.zone_actors.len(),
                    m.leaf_hulls.len(),
                    m.root_outside
                );
            }
        }
        for e in errors.iter().take(5) {
            out!("  {e}");
        }
    }
    out!("{ok} models read, {bad} errors");
    Ok(())
}

fn ray(game: &Path, map: &str, from: &str, dir: &str) -> Result<(), String> {
    let parse = |s: &str| -> Result<[f32; 3], String> {
        let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if v.len() == 3 {
            Ok([v[0], v[1], v[2]])
        } else {
            Err(format!("vector expected: {s}"))
        }
    };
    let (start, delta) = (parse(from)?, parse(dir)?);
    let map_path = game.join("Maps").join(format!("{map}.unr"));
    let data = std::fs::read(&map_path).map_err(|e| e.to_string())?;
    let package = Package::parse(&data).map_err(|e| e.to_string())?;
    let mut library = openxiii_package::library::Library::new(game).map_err(|e| e.to_string())?;
    library.insert(map, map_path.clone(), data, package);
    let world = openxiii_assets::world::build_world(&mut library, map).ok_or("geometry not found")?;
    let mesh = openxiii_assets::collision::CollisionMesh::new_tagged(world.collision, world.collision_tags);
    let end = [start[0] + delta[0], start[1] + delta[1], start[2] + delta[2]];
    let hit = mesh.sweep(start, end, 0.0);
    match hit {
        Some(h) => {
            let material = mesh.tags.get(h.triangle).and_then(|&t| world.collision_materials.get(t as usize)).cloned().unwrap_or_default();
            out!("contact at {:.1} %: {:?}, normal {:?}, material {material}", h.time * 100.0, h.location, h.normal);
            out!("triangle {:?}", mesh.triangles[h.triangle]);
        }
        None => out!("no contact"),
    }
    Ok(())
}

fn level(game: &Path, map: &Path, out: &Path) -> Result<(), String> {
    use openxiii_assets::level::{mesh_actors, world_obj};
    use openxiii_package::library::Library;
    let data = std::fs::read(map).map_err(|e| format!("{}: {e}", map.display()))?;
    let package = Package::parse(&data).map_err(|e| e.to_string())?;
    let name = map.file_stem().and_then(|s| s.to_str()).unwrap_or("map").to_string();
    let actors = mesh_actors(&name, &package, &data);
    let mut library = Library::new(game).map_err(|e| e.to_string())?;
    library.insert(&name, map.to_path_buf(), data, package);
    let mut world = world_obj(&mut library, &actors);
    let (map_package, map_data) = {
        let loaded = library.get(&name).ok_or("map not loaded")?;
        (loaded.package.clone(), loaded.data.clone())
    };
    let mut bsp_triangles = 0;
    match openxiii_assets::model::level_model(&map_package, &map_data) {
        Some(index) => {
            let model = openxiii_assets::model::Model::read(&map_package, &map_data, index).map_err(|e| format!("BSP: {e}"))?;
            let (obj, triangles) = openxiii_assets::level::bsp_obj(&mut library, &name, &map_package, &model, world.next_vertex, world.next_uv);
            world.obj += &obj;
            bsp_triangles = triangles;
            let next = world.next_vertex + model.polygons().iter().map(|(_, p)| p.len()).sum::<usize>();
            let (terrain, terrain_triangles, _) = openxiii_assets::level::terrain_obj(&mut library, &name, &map_package, &map_data, next);
            if terrain_triangles > 0 {
                world.obj += &terrain;
                out!("terrain: {terrain_triangles} triangles");
            }
            out!(
                "BSP {}: {} nodes, {} surfaces, {} points",
                map_package.object_path(ObjectRef::Export(index)),
                model.nodes.len(),
                model.surfs.len(),
                model.points.len()
            );
        }
        None => out!("no BSP model found"),
    }
    std::fs::write(out, &world.obj).map_err(|e| e.to_string())?;
    out!(
        "{} StaticMesh actors, {} placed, {} triangles; BSP {} triangles; {} meshes not found",
        actors.len(),
        world.placed,
        world.triangles,
        bsp_triangles,
        world.missing.len()
    );
    for m in world.missing.iter().take(10) {
        out!("  not found: {m}");
    }
    Ok(())
}

fn script_scan(dir: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect(dir, &mut files).map_err(|e| e.to_string())?;
    files.sort();
    let mut stats: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut tails: BTreeMap<String, BTreeMap<usize, usize>> = BTreeMap::new();
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples = Vec::new();
    let mut statements = 0usize;
    let mut conversions: BTreeMap<u8, usize> = BTreeMap::new();
    // Natives called (number): numbers < 0x70 or >= 0x100 go through a 0x60..0x6F token (HighNativeN).
    let mut natives: BTreeMap<u16, usize> = BTreeMap::new();
    for file in &files {
        let data = std::fs::read(file).map_err(|e| e.to_string())?;
        let p = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        for i in 0..p.exports.len() {
            let class = p.export_class(i);
            if !matches!(class, "Function" | "State" | "Class") || p.exports[i].serial_size == 0 {
                continue;
            }
            let entry = stats.entry(class.to_string()).or_default();
            match openxiii_script::read_struct(&p, &data, i) {
                Ok(o) => {
                    entry.0 += 1;
                    statements += o.script.len();
                    for e in &o.script {
                        e.walk(&mut |x| {
                            if let openxiii_script::bytecode::Kind::Conversion(t, _) = x.kind {
                                *conversions.entry(t).or_default() += 1;
                            }
                            if let openxiii_script::bytecode::Kind::NativeFunction(n, _) = x.kind {
                                *natives.entry(n).or_default() += 1;
                            }
                        });
                    }
                    *tails.entry(class.to_string()).or_default().entry(o.tail.len()).or_default() += 1;
                }
                Err(e) => {
                    entry.1 += 1;
                    let key = e.to_string().split(" at ").next().unwrap_or("").to_string();
                    *errors.entry(key).or_default() += 1;
                    if examples.len() < 200 {
                        examples.push(format!("{} {}: {e}", file.display(), p.object_path(ObjectRef::Export(i))));
                    }
                }
            }
        }
    }
    out!("{statements} statements read");
    out!("conversion tokens: {:?}", conversions.iter().map(|(t, n)| format!("{t:02X}:{n}")).collect::<Vec<_>>());
    let mut groups: BTreeMap<u16, usize> = BTreeMap::new();
    for (n, c) in &natives {
        *groups.entry(if *n >= 0x70 && *n < 0x100 { 0xFF } else { n >> 8 }).or_default() += c;
    }
    out!("native calls by token (0xFF: direct token 0x70..0xFF, N: HighNativeN): {groups:?}");
    for (class, (ok, ko)) in &stats {
        out!("{class:9} {ok:6} read, {ko:5} failed");
    }
    for (class, t) in &tails {
        let mut t: Vec<_> = t.iter().collect();
        t.sort_by(|a, b| b.1.cmp(a.1));
        out!("bytes after the script ({class}): {:?}", &t[..t.len().min(8)]);
    }
    let mut errors: Vec<_> = errors.into_iter().collect();
    errors.sort_by(|a, b| b.1.cmp(&a.1));
    for (e, n) in errors.iter().take(15) {
        out!("  {n:6} {e}");
    }
    for e in examples {
        out!("  {e}");
    }
    Ok(())
}

/// For tests: where each script package class's tagged properties
/// (defaultproperties) start and end.
fn defaults_offsets(path: &Path) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let p = Package::parse(&data).map_err(|e| e.to_string())?;
    for i in 0..p.exports.len() {
        if p.export_class(i) != "Class" {
            continue;
        }
        if let Ok(c) = openxiii_script::objects::read_class(&p, &data, i) {
            let end_value = p.exports[i].serial_offset as usize + p.exports[i].serial_size as usize;
            out!("{i} {} {} {end_value}", p.object_name(ObjectRef::Export(i)), c.defaults_offset);
        }
    }
    Ok(())
}

fn disasm(path: &Path, target: &str) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let p = Package::parse(&data).map_err(|e| e.to_string())?;
    let index = (0..p.exports.len())
        .find(|&i| p.object_path(ObjectRef::Export(i)).eq_ignore_ascii_case(target))
        .ok_or_else(|| format!("export not found: {target}"))?;
    out!("{} ({}) {} bytes @ 0x{:X}", target, p.export_class(index), p.exports[index].serial_size, p.exports[index].serial_offset);
    match openxiii_script::read_struct_partial(&p, &data, index) {
        Ok((o, error)) => {
            out!("{:?}", o.header);
            for e in &o.script {
                out!("{:5} {:?}", e.offset, e.kind);
            }
            match error {
                Some(e) => out!("error: {e}"),
                None => out!("end: {:02x?}", o.tail),
            }
            if p.export_class(index) == "Class" {
                match openxiii_script::objects::read_class(&p, &data, index) {
                    Ok(c) => out!(
                        "class: flags 0x{:X}, {} dependencies, {} imports, within {}, config {}, {} hidden categories, {} defaultproperties",
                        c.class_flags,
                        c.dependencies.len(),
                        c.package_imports.len(),
                        c.within,
                        p.name(c.config_name),
                        c.hide_categories.len(),
                        c.defaults.len()
                    ),
                    Err(e) => out!("class: error {e:?}"),
                }
            }
        }
        Err(e) => out!("error: {e}"),
    }
    Ok(())
}

/// For each class and state: ProbeMask, IgnoreMask and names of the functions defined in it
/// (used to find which bit corresponds to which "probe" event).
fn probes(dir: &Path) -> Result<(), String> {
    use openxiii_script::objects::*;
    let mut files = Vec::new();
    collect(dir, &mut files).map_err(|e| e.to_string())?;
    files.sort();
    for file in &files {
        let data = std::fs::read(file).map_err(|e| e.to_string())?;
        let p = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        for i in 0..p.exports.len() {
            let state = match p.export_class(i) {
                "State" => read_state(&p, &data, i).ok(),
                "Class" => read_class(&p, &data, i).ok().map(|c| c.state),
                _ => None,
            };
            let Some(state) = state else { continue };
            let functions: Vec<String> = (0..p.exports.len())
                .filter(|&j| p.export_class(j) == "Function" && p.exports[j].package == i as i32 + 1)
                .map(|j| p.name(p.exports[j].object_name).to_string())
                .collect();
            out!("{} {:016x} {:016x} {}", p.object_path((i as i32 + 1).into()), state.probe_mask, state.ignore_mask, functions.join(","));
        }
    }
    Ok(())
}

fn obj_scan(dir: &Path) -> Result<(), String> {
    use openxiii_script::objects::*;
    let mut files = Vec::new();
    collect(dir, &mut files).map_err(|e| e.to_string())?;
    files.sort();
    let mut stats: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut examples = Vec::new();
    for file in &files {
        let data = std::fs::read(file).map_err(|e| e.to_string())?;
        let p = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        for i in 0..p.exports.len() {
            let class = p.export_class(i);
            if p.exports[i].serial_size == 0 {
                continue;
            }
            let result = match class {
                "Function" => read_function(&p, &data, i).map(|_| ()),
                "State" => read_state(&p, &data, i).map(|_| ()),
                "Class" => read_class(&p, &data, i).map(|_| ()),
                "Enum" => read_enum(&p, &data, i).map(|_| ()),
                "Const" => read_const(&p, &data, i).map(|_| ()),
                c if c.ends_with("Property") => read_property(&p, &data, i).map(|_| ()),
                _ => continue,
            };
            let entry = stats.entry(class.to_string()).or_default();
            match result {
                Ok(()) => entry.0 += 1,
                Err(e) => {
                    entry.1 += 1;
                    if examples.len() < 15 {
                        examples.push(format!("{} {} ({class}): {e}", file.display(), p.object_path(ObjectRef::Export(i))));
                    }
                }
            }
        }
    }
    for (class, (ok, ko)) in &stats {
        out!("{class:20} {ok:7} read, {ko:5} failed");
    }
    for e in examples {
        out!("  {e}");
    }
    Ok(())
}

fn decompile(game: &Path, package: &Path, out: &Path) -> Result<(), String> {
    use openxiii_script::decompile::{Decompiler, Natives};
    let natives = Natives::from_dir(&openxiii_package::system_dir(&game));
    let data = std::fs::read(package).map_err(|e| format!("{}: {e}", package.display()))?;
    let p = Package::parse(&data).map_err(|e| e.to_string())?;
    let stem = package.file_stem().and_then(|s| s.to_str()).unwrap_or("package");
    let dir = out.join(stem).join("Classes");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let d = Decompiler { package_name: stem, p: &p, data: &data, natives: &natives };
    let (mut written, mut failed) = (0, 0);
    for i in 0..p.exports.len() {
        if p.export_class(i) != "Class" || p.exports[i].serial_size == 0 {
            continue;
        }
        match openxiii_script::objects::read_class(&p, &data, i) {
            Ok(c) => {
                let name = p.name(p.exports[i].object_name);
                std::fs::write(dir.join(format!("{name}.uc")), d.class(i, &c)).map_err(|e| e.to_string())?;
                written += 1;
            }
            Err(e) => {
                failed += 1;
                out!("failed {}: {e}", p.object_path(ObjectRef::Export(i)));
            }
        }
    }
    out!("{written} classes decompiled in {} ({failed} failures, {} known natives)", dir.display(), natives.0.len());
    Ok(())
}

fn load_vm(game: &Path) -> Result<openxiii_vm::Vm, String> {
    let started = std::time::Instant::now();
    let mut vm = openxiii_vm::Vm::new();
    // No network unless OXIII_NETWORK=1: simulated sockets.
    vm.ipdrv.offline = openxiii_package::env::var("OXIII_NETWORK").map_or(true, |v| v != "1");
    // Map downloads allowed (deliberate difference, net_file.rs).
    vm.ipdrv.downloads = true;
    vm.load_dir(&openxiii_package::system_dir(&game)).map_err(|e| e.to_string())?;
    out!(
        "VM: {} packages, {} classes, {} functions loaded in {:.2} s",
        vm.packages.len(),
        vm.classes.len(),
        vm.functions.len(),
        started.elapsed().as_secs_f32()
    );
    Ok(vm)
}

fn print_vm_output(vm: &mut openxiii_vm::Vm) {
    for line in vm.log.drain(..) {
        out!("  log  {line}");
    }
    for w in vm.warnings.drain(..) {
        out!("  WARNING {w}");
    }
}

fn vm_test(game: &Path) -> Result<(), String> {
    let mut vm = load_vm(game)?;
    let class = vm.class("TestInfo").ok_or("class TestInfo missing")?;
    let obj = vm.spawn(class);
    // Suite failure: a function in error (assertion included) or no "All tests passed"
    // (PostBeginPlay logs it after all the tests); the exit code reports it.
    let mut errors_list = Vec::new();
    let mut succeeded = false;
    for f in ["TestQ", "TestStructBools", "TestContinueFor", "TestContinueWhile", "TestOptionalOut", "TestSwitch", "PostBeginPlay", "BeginPlay", "Tick"] {
        let args = if f == "Tick" { vec![openxiii_vm::value::Value::Float(0.1)] } else { vec![] };
        match vm.call_named(obj, f, args) {
            Ok(_) => out!("{f}: executed"),
            Err(e) => {
                out!("{f}: ERROR {e}");
                errors_list.push(f);
            }
        }
        succeeded |= vm.log.iter().any(|l| l.ends_with("All tests passed"));
        print_vm_output(&mut vm);
    }
    if !errors_list.is_empty() {
        return Err(format!("vmtest: FAILED, error in {}", errors_list.join(", ")));
    }
    if !succeeded {
        return Err("vmtest: FAILED, \"All tests passed\" missing".into());
    }
    Ok(())
}

fn vm_run(game: &Path, class: &str, function: &str, args: &[String]) -> Result<(), String> {
    use openxiii_vm::value::Value;
    let mut vm = load_vm(game)?;
    let c = vm.class(class).ok_or_else(|| format!("class {class} missing"))?;
    let obj = vm.spawn(c);
    let values: Vec<Value> = args
        .iter()
        .map(|a| {
            if let Ok(i) = a.parse::<i32>() {
                Value::Int(i)
            } else if let Ok(f) = a.parse::<f32>() {
                Value::Float(f)
            } else {
                Value::Str(a.clone())
            }
        })
        .collect();
    let result = vm.call_named(obj, function, values).map_err(|e| e.to_string());
    print_vm_output(&mut vm);
    out!("result: {:?}", result?);
    Ok(())
}

/// Runs a named handler of a sound program on a test actor and traces the voices.
fn hx_run(game: &Path, map: &str, handler: &str, seconds: f64, params: &[String]) -> Result<(), String> {
    use openxiii_assets::hx::Cuuid;
    use openxiii_hx::engine::{ActorRes, ObjKind};
    let mut e = openxiii_hx::Engine::new();
    let banks = e.load_banks(game, &[map, "Fix"]);
    out!("banks: {banks:?}, {} resources", e.registry.len());
    let prog_id = e.find_handler_program(handler).ok_or_else(|| format!("handler {handler} not found"))?;
    out!("program {prog_id}");
    let actor_res = Cuuid(0xac70, 1);
    e.add_actor_res(actor_res, ActorRes { name: "Test".into(), is3d: false, rolloff: [0.0, 2000.0, 100.0, -100.0] });
    let actor = e.create_object(actor_res).ok_or("actor")?;
    let prog = e.create_object(prog_id).ok_or("program")?;
    e.actor_attach_top(actor, prog, 0, 0, false);
    let index = e.program_obj(prog).and_then(|p| p.prog.handler_index(handler.as_bytes())).ok_or("handler")? as u32;
    let words: Vec<u32> = params.iter().map(|s| s.parse::<i32>().unwrap_or(0) as u32).collect();
    let mut bytes = Vec::new();
    for w in &words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    let block = if bytes.is_empty() { 0 } else { e.mem.alloc_with(&bytes) };
    e.exec_handler(prog, index, block);
    let names: std::collections::HashMap<Cuuid, String> = e
        .registry
        .iter()
        .filter_map(|(id, r)| match r {
            openxiii_hx::engine::ResData::Wav(w) => Some((*id, w.name.clone())),
            _ => None,
        })
        .collect();
    let mut seen: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
    let step = 0.02;
    let mut t = 0.0;
    let mut last_pos: std::collections::HashMap<usize, Option<u64>> = std::collections::HashMap::new();
    while t <= seconds {
        // Streamed waves: what is audible (stream position under the channel's cursor).
        for (i, v) in e.voices.iter().enumerate() {
            let Some(v) = v else { continue };
            let Some(h) = &v.stream else { continue };
            let (now, freq) = match h.lock() {
                Ok(s) => (s.chan.audible(), s.chan.frequency()),
                Err(_) => continue,
            };
            let prev = last_pos.insert(i, now).flatten();
            let r = v.rate.max(1) as f64;
            match (prev, now) {
                (None, Some(p)) => out!("{:7.3} s  voice {i}: audible at {:.3} s of the stream", t, p as f64 / r),
                (Some(a), None) => out!("{:7.3} s  voice {i}: silent (after {:.3} s of the stream)", t, a as f64 / r),
                (Some(a), Some(b)) => {
                    let expect = a as f64 + step * freq;
                    if (b as f64 - expect).abs() > freq * step * 0.5 {
                        out!("{:7.3} s  voice {i}: jump {:.3} s -> {:.3} s", t, a as f64 / r, b as f64 / r);
                    }
                }
                _ => {}
            }
        }
        for (i, v) in e.voices.iter().enumerate() {
            let Some(v) = v else { continue };
            if v.playing && seen.get(&v.serial) != Some(&v.starts) {
                seen.insert(v.serial, v.starts);
                out!(
                    "{:7.3} s  voice {i}: {} gain {:.3} pitch {:.4} pan {:.2} {}",
                    t,
                    names.get(&v.res).map_or("?", |s| s.as_str()),
                    v.gain,
                    v.pitch,
                    v.pan,
                    if v.looping { "loop" } else { "" }
                );
            }
        }
        e.time_ms += (step * 1000.0) as u32;
        // The decoder thread's passes of this step (streamed waves get their frames from them).
        e.stream_threads(step);
        e.advance_static_voices(step);
        e.advance_voices(step);
        e.tick_timers();
        e.actor_update(actor);
        t += step;
    }
    let live = e.actor(actor).map_or(0, |a| a.entries.len());
    out!("end: {live} object(s) still attached");
    for l in e.logs.drain(..) {
        out!("log: {l}");
    }
    let _ = matches!(e.object(prog).map(|o| &o.kind), Some(ObjKind::Program(_)));
    Ok(())
}

fn sound_to_wav(game: &Path, map: &str, sound: &str, out: &Path) -> Result<(), String> {
    let mut library = openxiii_package::library::Library::new(game).map_err(|e| e.to_string())?;
    let id = openxiii_assets::audio::sound_id(&mut library, sound).ok_or("Sound object not found or without an HX resource")?;
    let mut set = openxiii_assets::audio::SoundSet::load(game, &[map, "Fix"]);
    out!("{} sounds in the {map} and Fix banks; {sound} -> {id}", set.len());
    let pcm = set.pcm(id, 0).ok_or("resource missing from the banks or codec not supported")?;
    let bytes: Vec<u8> = pcm.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    openxiii_assets::hx::write_wav(out, pcm.channels, pcm.rate, &bytes).map_err(|e| e.to_string())?;
    out!("{} samples, {} channels, {} Hz -> {}", pcm.samples.len(), pcm.channels, pcm.rate, out.display());
    Ok(())
}

/// All the classes as JSON (C++ name with A/U prefix, size, fields).
fn layouts_json(game: &Path) -> Result<(), String> {
    let mut vm = openxiii_vm::Vm::new();
    // No network unless OXIII_NETWORK=1: simulated sockets.
    vm.ipdrv.offline = openxiii_package::env::var("OXIII_NETWORK").map_or(true, |v| v != "1");
    vm.load_dir(&openxiii_package::system_dir(&game)).map_err(|e| e.to_string())?;
    let actor = vm.class("Actor");
    let names: Vec<(usize, String)> = vm.classes.iter().enumerate().map(|(i, c)| (i, c.name.clone())).collect();
    let mut out = String::from(
        "[
",
    );
    let mut first = true;
    for (ci, name) in names {
        let Some((fields, size)) = vm.class_layout(&name) else {
            continue;
        };
        let prefix = if actor.is_some_and(|a| vm.is_child_of(ci, a)) { "A" } else { "U" };
        if !first {
            out += ",
";
        }
        first = false;
        out += &format!("{{\"name\":\"{prefix}{name}\",\"size\":{size},\"fields\":[");
        let mut seen = std::collections::HashSet::new();
        let mut parts = Vec::new();
        for f in &fields {
            // Booleans of the same word share the position: only one name is kept.
            if !seen.insert(f.offset) {
                continue;
            }
            let ty = f.ty.replace('"', "");
            let fname = if f.bit.is_some() { format!("bits_{}", f.name) } else { f.name.clone() };
            parts.push(format!("{{\"name\":\"{fname}\",\"offset\":{},\"size\":{},\"type\":\"{ty}\"}}", f.offset, f.size));
        }
        out += &parts.join(",");
        out += "]}";
    }
    out += "
]
";
    print!("{out}");
    Ok(())
}

fn layout(game: &Path, class: &str) -> Result<(), String> {
    let mut vm = openxiii_vm::Vm::new();
    // No network unless OXIII_NETWORK=1: simulated sockets.
    vm.ipdrv.offline = openxiii_package::env::var("OXIII_NETWORK").map_or(true, |v| v != "1");
    vm.load_dir(&openxiii_package::system_dir(&game)).map_err(|e| e.to_string())?;
    let (fields, size) = vm.class_layout(class).ok_or(format!("unknown class: {class}"))?;
    for f in &fields {
        let bit = f.bit.map(|b| format!(" bit {b:#x}")).unwrap_or_default();
        out!("{:#06x} {:>4} {:<14} {}.{}: {}", f.offset, f.size, bit, f.owner, f.name, f.ty);
    }
    out!("size {size:#x}");
    Ok(())
}

/// Collision geometry of the loaded map, built as for the viewer (BSP and
/// fixed StaticMeshes, Mover shapes apart).
pub(crate) fn prepare_collisions(vm: &mut openxiii_vm::Vm, game: &Path, map: &str, map_path: &Path) -> Result<(), String> {
    let data = std::fs::read(&map_path).map_err(|e| e.to_string())?;
    let package = Package::parse(&data).map_err(|e| e.to_string())?;
    let mut library = openxiii_package::library::Library::new(game).map_err(|e| e.to_string())?;
    library.insert(map, map_path.to_path_buf(), data, package);
    // Like the viewer in play mode: Movers (doors, gates…) have their own collision
    // shape that follows their movements, outside the fixed geometry.
    let mover_class = vm.class("Mover");
    let movers: Vec<usize> =
        vm.actors.iter().copied().filter(|&a| mover_class.is_some_and(|m| vm.objects[a].as_ref().is_some_and(|o| vm.is_child_of(o.class, m)))).collect();
    let names: std::collections::HashSet<String> = movers.iter().filter_map(|&a| vm.objects[a].as_ref().map(|o| o.name.to_lowercase())).collect();
    if let Some(world) = openxiii_assets::world::build_world_excluding(&mut library, map, &names) {
        let triangles = world.collision.len();
        let tags = vm.adopt_world_materials(&world.collision_materials, &world.collision_tags);
        vm.collision = Some(openxiii_assets::collision::CollisionMesh::new_tagged(world.collision, tags));
        out!("collisions: {triangles} blocking triangles");
    }
    for a in movers {
        if let openxiii_vm::value::Value::Asset(mesh) = vm.prop(a, "StaticMesh") {
            if let Some(tris) = openxiii_assets::world::static_mesh_triangles(&mut library, &mesh) {
                vm.set_mover_shape(a, tris);
            }
        }
    }
    Ok(())
}

fn skeletal(input: &Path, out: Option<&Path>) -> Result<(), String> {
    use openxiii_assets::skeletal::SkeletalMesh;
    let mut files = Vec::new();
    if input.is_dir() {
        collect(input, &mut files).map_err(|e| e.to_string())?;
    } else {
        files.push(input.to_path_buf());
    }
    files.sort();
    let (mut ok, mut failed, mut faces, mut bones) = (0, 0, 0, 0);
    let mut versions: BTreeMap<u32, usize> = BTreeMap::new();
    let mut errors = Vec::new();
    for file in &files {
        let data = std::fs::read(file).map_err(|e| e.to_string())?;
        let p = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("package");
        for i in 0..p.exports.len() {
            if p.export_class(i) != "SkeletalMesh" || p.exports[i].serial_size == 0 {
                continue;
            }
            match SkeletalMesh::read(&p, &data, i) {
                Ok(mesh) => {
                    ok += 1;
                    faces += mesh.faces.len();
                    bones += mesh.bones.len();
                    *versions.entry(mesh.version).or_default() += 1;
                    if let Some(out) = out {
                        let dir = out.join(stem);
                        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                        let name = p.object_path(ObjectRef::Export(i)).replace(['/', '\\', ':'], "_");
                        let material = |r: i32| -> String {
                            match ObjectRef::from(r) {
                                ObjectRef::None => "none".into(),
                                ObjectRef::Export(_) => {
                                    format!("{stem}.{}", p.object_path(r.into()))
                                }
                                _ => p.object_path(r.into()),
                            }
                        };
                        std::fs::write(dir.join(format!("{name}.obj")), mesh.to_obj(&material)).map_err(|e| e.to_string())?;
                    }
                }
                Err(e) => {
                    failed += 1;
                    if errors.len() < 15 {
                        errors.push(format!("{} {}: {e}", file.display(), p.object_path(ObjectRef::Export(i))));
                    }
                }
            }
        }
    }
    out!("{ok} SkeletalMesh read, {failed} failed; {faces} faces, {bones} bones; versions {versions:?}");
    for e in errors {
        out!("  {e}");
    }
    Ok(())
}

fn anims(input: &Path) -> Result<(), String> {
    use openxiii_assets::animation::MeshAnimation;
    let mut files = Vec::new();
    if input.is_dir() {
        collect(input, &mut files).map_err(|e| e.to_string())?;
    } else {
        files.push(input.to_path_buf());
    }
    files.sort();
    let (mut ok, mut failed, mut sequences, mut keys) = (0, 0, 0, 0);
    let mut errors = Vec::new();
    for file in &files {
        let data = std::fs::read(file).map_err(|e| e.to_string())?;
        let p = Package::parse(&data).map_err(|e| format!("{}: {e}", file.display()))?;
        for i in 0..p.exports.len() {
            if p.export_class(i) != "MeshAnimation" || p.exports[i].serial_size == 0 {
                continue;
            }
            match MeshAnimation::read(&p, &data, i) {
                Ok(a) => {
                    ok += 1;
                    sequences += a.sequences.len();
                    keys += a.moves.iter().map(|m| m.quats.len()).sum::<usize>();
                }
                Err(e) => {
                    failed += 1;
                    if errors.len() < 15 {
                        errors.push(format!("{} {}: {e}", file.display(), p.object_path(ObjectRef::Export(i))));
                    }
                }
            }
        }
    }
    out!("{ok} MeshAnimation read, {failed} failed; {sequences} sequences, {keys} rotation keys");
    for e in errors {
        out!("  {e}");
    }
    Ok(())
}

fn pose(game: &Path, mesh_path: &str, sequence: &str, frame: f32, out: &Path) -> Result<(), String> {
    use openxiii_assets::{animation::MeshAnimation, skeletal::SkeletalMesh};
    let mut library = openxiii_package::library::Library::new(game).map_err(|e| e.to_string())?;
    let (mesh, mesh_pkg, anim_path) = {
        let (loaded, index) = library.resolve(mesh_path).ok_or("mesh not found")?;
        let mesh = SkeletalMesh::read(&loaded.package, &loaded.data, index).map_err(|e| e.to_string())?;
        let anim_path = match ObjectRef::from(mesh.animation) {
            ObjectRef::None => return Err("no animation".into()),
            ObjectRef::Export(_) => format!("{}.{}", loaded.name, loaded.package.object_path(mesh.animation.into())),
            _ => loaded.package.object_path(mesh.animation.into()),
        };
        (mesh, loaded.name.clone(), anim_path)
    };
    let bone_names: Vec<String> = {
        let loaded = library.get(&mesh_pkg).ok_or("package")?;
        mesh.bones.iter().map(|b| loaded.package.name(b.name).to_string()).collect()
    };
    let (loaded, index) = library.resolve(&anim_path).ok_or_else(|| format!("animation not found: {anim_path}"))?;
    let anim = MeshAnimation::read(&loaded.package, &loaded.data, index).map_err(|e| e.to_string())?;
    let names: Vec<String> = anim.sequences.iter().map(|s| loaded.package.name(s.name).to_string()).collect();
    let seq = names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(sequence))
        .or_else(|| sequence.parse().ok())
        .ok_or_else(|| format!("unknown sequence; available: {}", names.join(", ")))?;
    let pkg = &loaded.package;
    let mesh_bone_name = |n: i32| bone_names.get(mesh.bones.iter().position(|b| b.name == n).unwrap_or(0)).cloned().unwrap_or_default();
    let points = mesh.pose(&anim, &|n| pkg.name(n).to_string(), &mesh_bone_name, seq, frame);
    let posed = mesh.with_points(points);
    let material = |r: i32| -> String {
        match ObjectRef::from(r) {
            ObjectRef::None => "none".into(),
            _ => format!("{}", r),
        }
    };
    let _ = material;
    let mesh_loaded = library.get(&mesh_pkg).ok_or("package")?;
    let mp = &mesh_loaded.package;
    let name_of = |r: i32| -> String {
        match ObjectRef::from(r) {
            ObjectRef::None => "none".into(),
            ObjectRef::Export(_) => format!("{}.{}", mesh_pkg, mp.object_path(r.into())),
            _ => mp.object_path(r.into()),
        }
    };
    std::fs::write(out, posed.to_obj(&name_of)).map_err(|e| e.to_string())?;
    out!(
        "{}: animation {anim_path}, {} moves, sequence {} ({}), frame {frame}, {} frames",
        mesh_path,
        anim.moves.len(),
        names[seq],
        seq,
        anim.sequences[seq].num_frames
    );
    out!("  MeshScale {:?} MeshOrigin {:?} RotOrigin {:?}", posed.scale, posed.origin, posed.rot_origin);
    if let Some(root) = mesh.bones.first() {
        out!("  root bone {}: orientation {:?} position {:?}", bone_names.first().cloned().unwrap_or_default(), root.orientation, root.position);
    }
    if let Some(m) = anim.moves.get(seq) {
        if let Some(t) = m.tracks.first() {
            out!(
                "  track 0 (bone {}): first quaternion {:?}, first position {:?}",
                m.bone_indices.first().copied().unwrap_or(0),
                m.quats.get(t.quat_start as usize),
                m.positions.get(t.pos_start as usize)
            );
        }
    }
    // Bounding boxes (actor frame): reference pose and animated pose.
    let bbox = |m: &openxiii_assets::skeletal::SkeletalMesh| {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for i in 0..m.points.len() {
            let q = m.transformed_point(i);
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        (lo, hi)
    };
    out!("  reference pose box {:?}", bbox(&mesh));
    out!("  animated pose box {:?}", bbox(&posed));
    Ok(())
}
