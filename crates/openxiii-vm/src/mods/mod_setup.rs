//! Mods build only: discovery of the mods at startup and hand-over of everything they define to
//! the engine (class values, new classes, map changes, actions and characters, sounds,
//! textures, 3D models, network identity), before any game script is loaded. Used by the game
//! window and by the command-line tools (dedicated server, simulated client).

use std::collections::HashMap;
use std::sync::Arc;

/// Discovers the mods (unless `--no-mods` is in `args` or OXIII_NO_MODS=1) and hands them to
/// the engine. `game` is the original game's folder, where the mods' recipes read the game's
/// meshes and textures (`[arms.<Name>]`); without it they are skipped. `out` receives the lines for the log, each starting with "mods: ". When the Mods
/// folder exists, the same lines (and those of the rest of the run) also go to its
/// mods-log.txt.
pub fn start_mods(args: &[String], game: Option<&std::path::Path>, out: &mut dyn FnMut(String)) {
    let mut say = |s: String| {
        let line = format!("mods: {s}");
        openxiii_mods::log::note(&line);
        out(line)
    };
    if openxiii_mods::disabled(args) {
        say("disabled".into());
        return;
    }
    let Some(folder) = openxiii_mods::mods_folder() else {
        say("no Mods folder".into());
        openxiii_mods::set_loaded_at_start(Vec::new());
        return;
    };
    let found = openxiii_mods::discover(&folder);
    // What this run loaded: the Mods page marks the boxes changed since (a restart applies them).
    openxiii_mods::set_loaded_at_start(found.mods.iter().map(|m| m.folder_name.clone()).collect());
    openxiii_mods::log::start(&folder, &found);
    for p in &found.problems {
        say(p.to_string());
    }
    for d in &found.disabled {
        say(format!("folder {d} turned off ({})", openxiii_mods::DISABLED_FILE));
    }
    for m in &found.mods {
        say(format!("loaded \"{}\" version {} (folder {})", m.info.name, m.info.version, m.folder.display()));
    }
    say(format!("{} mod(s) loaded from {}", found.mods.len(), folder.display()));
    // Network identity: the name, version and fingerprint of each mod (mod_net.rs).
    let ids = openxiii_mods::mod_ids(&found.mods);
    for id in &ids {
        say(format!("network identity of mod {}", id.label()));
    }
    // Objects the mods add, which the network designates in the "Mod" package.
    let mut objects: Vec<(String, &'static str)> = Vec::new();
    let (overrides, conflicts) = openxiii_mods::collect_overrides(&found.mods);
    for p in &conflicts {
        say(p.to_string());
    }
    if !overrides.is_empty() {
        say(format!("{} class default value(s) set by mods", overrides.len()));
    }
    crate::mod_defaults::set_mod_defaults(overrides);
    let (classes, conflicts) = openxiii_mods::collect_new_classes(&found.mods);
    for p in &conflicts {
        say(p.to_string());
    }
    objects.extend(classes.iter().map(|c| (c.class_name(), "Class")));
    crate::mod_classes::set_mod_classes(classes);
    let (edits, conflicts) = openxiii_mods::collect_edits(&found.mods);
    for p in &conflicts {
        say(p.to_string());
    }
    if !edits.is_empty() {
        say(format!("{} map change(s) set by mods", edits.len()));
    }
    let (actions, mut characters, conflicts) = openxiii_mods::collect_behaviour(&found.mods);
    for p in &conflicts {
        say(p.to_string());
    }
    // Simple words: look names as characters, entries' simple keys over their character.
    for p in openxiii_mods::simple::resolve(&edits, &actions, &mut characters) {
        say(p.to_string());
    }
    // "when ... then ..." blocks: rules of the scripts' engine (mod_when.rs).
    let rules = when_rules(&edits);
    for (rule, about) in &rules {
        say(format!("{about} on {} ({})", rule.map, rule.source));
    }
    // Keys the game binds by default (its DefUser.ini): a shortcut or a key reaction there is
    // left to the game, said now rather than when the key is pressed.
    if let Some(game) = game {
        let defaults = std::fs::read_to_string(game.join("System").join("DefUser.ini")).unwrap_or_default();
        let mut keys: Vec<(String, String)> = actions.iter().map(|a| (a.key.clone(), a.source.to_string())).collect();
        keys.extend(rules.iter().filter_map(|(r, _)| match &r.event {
            crate::mod_when::WhenEvent::Key { key } => Some((key.clone(), r.source.clone())),
            _ => None,
        }));
        for line in game_bound_keys(&defaults, &keys) {
            say(line);
        }
    }
    crate::mod_when::set_mod_when(rules.into_iter().map(|(r, _)| r).collect());
    crate::mod_placement::set_mod_edits(edits);
    for a in &actions {
        say(format!("keyboard shortcut {} ({})", a.key, a.source));
    }
    crate::mod_behaviour::set_mod_behaviour(actions, characters);
    // The mods with Lua scripts: their scripts folders are read when a map starts.
    let scripted: Vec<crate::mod_script::ScriptMod> = found
        .mods
        .iter()
        .filter(|m| !m.scripts.is_empty())
        .map(|m| crate::mod_script::ScriptMod { name: m.folder_name.clone(), folder: m.folder.clone() })
        .collect();
    for s in found.mods.iter().flat_map(|m| m.scripts.iter()) {
        say(format!("script {}", s.label));
    }
    crate::mod_script::set_mod_scripts(scripted);
    // The mods' own maps and the packages they need, found by name after the game's own
    // (openxiii-package, library::map_path); the first mod that brings a name keeps it.
    let mut extra: Vec<(String, std::path::PathBuf)> = Vec::new();
    for p in found.mods.iter().flat_map(|m| openxiii_mods::own_maps::find(&m.folder, &m.folder_name)) {
        if extra.iter().any(|(n, _)| n.eq_ignore_ascii_case(&p.name)) {
            say(format!("warning: {} {} of mod {} is already brought by another mod; this one is not used", if p.map { "map" } else { "package" }, p.name, p.mod_name));
            continue;
        }
        if p.map {
            say(format!("map {} (mod {})", p.name, p.mod_name));
        }
        extra.push((p.name, p.path));
    }
    crate::mod_maps::set_mod_map_files(extra.clone());
    openxiii_package::library::set_extra_files(extra);
    crate::mod_sounds::set_mod_loads(found.mods.iter().flat_map(|m| m.loads.clone()).collect());
    let sounds: Vec<openxiii_mods::own_sounds::OwnSound> = found.mods.iter().flat_map(|m| m.sounds.clone()).collect();
    objects.extend(sounds.iter().map(|s| (format!("{}{}", openxiii_mods::own_models::PREFIX, s.name), "Sound")));
    for line in crate::mod_own_sounds::set_mod_own_sounds(sounds) {
        say(line);
    }
    // The mods' own textures: decoded once; each one under "Mod.<Name>" and under the game
    // texture it replaces.
    let mut textures = HashMap::new();
    let mut clamped = std::collections::HashSet::new();
    for t in found.mods.iter().flat_map(|m| m.textures.iter()) {
        if textures.contains_key(&t.texture_name().to_ascii_lowercase()) {
            say(format!("warning: texture {} defined twice, the first one is kept ({})", t.texture_name(), t.source));
            continue;
        }
        if t.clamp {
            clamped.insert(t.texture_name());
        }
        match openxiii_mods::own_textures::decode_png(&t.file) {
            Ok(m) => {
                let m = Arc::new(m);
                textures.insert(t.texture_name().to_ascii_lowercase(), m.clone());
                objects.push((t.texture_name(), "Texture"));
                if let Some(r) = &t.replace {
                    textures.insert(r.to_ascii_lowercase(), m);
                    say(format!("texture {} is drawn instead of {r} ({})", t.texture_name(), t.source));
                }
            }
            Err(e) => say(format!("warning: texture {}: {e}; texture skipped ({})", t.texture_name(), t.source)),
        }
    }
    // The mods' own 3D models: read once; each one under "Mod.<Name>" and under the game mesh it
    // replaces. The PNG texture of each part becomes the texture "Mod.<Name>.<part>".
    let mut meshes = HashMap::new();
    let (mut skeletal, mut animations) = (HashMap::new(), HashMap::new());
    for m in found.mods.iter().flat_map(|m| m.models.iter()) {
        if meshes.contains_key(&m.mesh_name().to_ascii_lowercase()) || skeletal.contains_key(&m.mesh_name().to_ascii_lowercase()) {
            say(format!("warning: model {} defined twice, the first one is kept ({})", m.mesh_name(), m.source));
            continue;
        }
        // A model with a skin is an animated model (a skeletal mesh of the game).
        if openxiii_mods::own_animated::is_animated(&m.file) {
            let mut lines = Vec::new();
            let built = crate::mod_models::build_animated_model(m, &mut textures, &mut lines);
            for l in lines {
                say(l);
            }
            match built {
                Ok((mut mesh, animation)) => {
                    mesh.cartoon = openxiii_assets::mod_meshes::cartoon_bytes(m.outline, m.cartoon, m.outline_width);
                    let mesh = Arc::new(mesh);
                    objects.push((m.mesh_name(), "SkeletalMesh"));
                    objects.push((mesh.animation_name.clone(), "MeshAnimation"));
                    animations.insert(mesh.animation_name.to_ascii_lowercase(), Arc::new(animation));
                    skeletal.insert(m.mesh_name().to_ascii_lowercase(), mesh.clone());
                    if let Some(r) = &m.replace {
                        skeletal.insert(r.to_ascii_lowercase(), mesh);
                        say(format!("animated model {} is drawn instead of {r} ({})", m.mesh_name(), m.source));
                    }
                }
                Err(e) => say(format!("warning: model {}: {e}; model skipped ({})", m.mesh_name(), m.source)),
            }
            continue;
        }
        if !m.animations.is_empty() {
            say(format!("warning: model {}: animations are given but the file has no skin; they are ignored ({})", m.mesh_name(), m.source));
        }
        let parts = match openxiii_mods::own_models::load_gltf(&m.file, m.scale) {
            Ok(p) => p,
            Err(e) => {
                say(format!("warning: model {}: {e}; model skipped ({})", m.mesh_name(), m.source));
                continue;
            }
        };
        let mut built = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            // A game material named by the mod is drawn as the game draws it; otherwise the
            // model's own PNG becomes the new texture "Mod.<Name>.<part>".
            let mut material = m.material.clone().unwrap_or_default();
            if let (Some(png), None) = (&p.texture, &m.material) {
                match openxiii_mods::own_textures::decode_png(png) {
                    Ok(t) => {
                        material = format!("{}.{i}", m.mesh_name());
                        textures.insert(material.to_ascii_lowercase(), Arc::new(t));
                    }
                    Err(e) => say(format!("warning: model {}: texture {e}; part drawn without it ({})", m.mesh_name(), m.source)),
                }
            }
            built.push(openxiii_assets::mod_meshes::Part { positions: &p.positions, normals: &p.normals, uvs: &p.uvs, indices: &p.indices, material });
        }
        match openxiii_assets::mod_meshes::build(&built) {
            Ok((mut mesh, materials)) => {
                // Outline and flat shading as on the game's objects, unless the mod turns them off.
                mesh.unknown = openxiii_assets::mod_meshes::cartoon_bytes(m.outline, m.cartoon, m.outline_width);
                let collision = match m.collision {
                    openxiii_mods::own_models::Collision::Mesh => openxiii_assets::mod_meshes::Collision::Mesh,
                    openxiii_mods::own_models::Collision::Box => openxiii_assets::mod_meshes::Collision::Box,
                    openxiii_mods::own_models::Collision::None => openxiii_assets::mod_meshes::Collision::None,
                };
                let mesh = Arc::new((mesh, materials, collision));
                objects.push((m.mesh_name(), "StaticMesh"));
                meshes.insert(m.mesh_name().to_ascii_lowercase(), mesh.clone());
                if let Some(r) = &m.replace {
                    meshes.insert(r.to_ascii_lowercase(), mesh);
                    say(format!("model {} is drawn instead of {r} ({})", m.mesh_name(), m.source));
                }
                say(format!("model {} read ({} part(s)) ({})", m.mesh_name(), parts.len(), m.source));
            }
            Err(e) => say(format!("warning: model {}: {e}; model skipped ({})", m.mesh_name(), m.source)),
        }
    }
    // First-person arms made from a game mesh (`[arms.<Name>]`): a recipe applied to the
    // player's own game files (openxiii_assets::mod_arms), nothing of the game in the mod.
    let arms: Vec<&openxiii_mods::own_arms::OwnArms> = found.mods.iter().flat_map(|m| m.arms.iter()).collect();
    if !arms.is_empty() {
        match game.map(openxiii_package::library::Library::new) {
            Some(Ok(mut library)) => {
                for a in arms {
                    let name = a.mesh_name();
                    if meshes.contains_key(&name.to_ascii_lowercase()) || skeletal.contains_key(&name.to_ascii_lowercase()) {
                        say(format!("warning: model {name} defined twice, the first one is kept ({})", a.source));
                        continue;
                    }
                    let recipe = arms_recipe(a);
                    let started = std::time::Instant::now();
                    match openxiii_assets::mod_arms::build(&mut library, &name, &recipe) {
                        Ok(built) => {
                            let ms = started.elapsed().as_secs_f64() * 1000.0;
                            for (texture, w, h, rgba) in built.textures {
                                textures.insert(texture.to_ascii_lowercase(), Arc::new(openxiii_mods::own_textures::mips(w, h, rgba)));
                                objects.push((texture, "Texture"));
                            }
                            for n in built.notes {
                                say(format!("arms {name}: {n}"));
                            }
                            // "Mod.<Name>" for a single mesh named as is; every mesh also under
                            // "Mod.<Name>.<Mesh>", and under the game's mesh with replace.
                            let single = built.meshes.len() == 1 && !a.from[0].ends_with(".*");
                            let count = built.meshes.len();
                            for (from, mesh) in built.meshes {
                                let mesh = Arc::new(mesh);
                                let own = format!("{name}.{}", from.rsplit('.').next().unwrap_or(&from));
                                let mut names = vec![own];
                                if single {
                                    names.push(name.clone());
                                }
                                for n in names {
                                    objects.push((n.clone(), "SkeletalMesh"));
                                    skeletal.insert(n.to_ascii_lowercase(), mesh.clone());
                                }
                                if a.replace {
                                    skeletal.insert(from.to_ascii_lowercase(), mesh);
                                }
                            }
                            let how = if a.replace { ", drawn instead of the game's" } else { "" };
                            say(format!("arms {name}: {count} first-person mesh(es) made from {} in {ms:.1} ms{how} ({})", a.from.join(", "), a.source));
                        }
                        Err(e) => say(format!("warning: arms {name}: {e}; arms skipped ({})", a.source)),
                    }
                }
            }
            _ => say("warning: the game's folder could not be read: the mods' arms are skipped".into()),
        }
    }
    openxiii_assets::mod_meshes::set(meshes);
    openxiii_assets::mod_skeletal::set(skeletal, animations);
    // The HUD icon of a mod's new weapon that gives none (mod_weapons.rs).
    textures.entry(crate::mod_weapons::DEFAULT_ICON.to_string()).or_insert_with(|| Arc::new(crate::mod_weapons::default_icon()));
    openxiii_assets::mod_textures::set(textures);
    openxiii_assets::mod_textures::set_clamped(clamped);
    crate::mod_net::set_mod_network(ids, objects);
}

/// The building recipe of an `[arms.<Name>]` table (openxiii_assets::mod_arms).
pub(crate) fn arms_recipe(a: &openxiii_mods::own_arms::OwnArms) -> openxiii_assets::mod_arms::Recipe {
    use openxiii_assets::mod_arms::{Recipe, Shape, Skin};
    use openxiii_mods::own_arms::Skin as Asked;
    let skin = |s: &Asked| match s {
        Asked::Unchanged => Skin::Unchanged,
        Asked::Color(c) => Skin::Color(*c),
        Asked::From(m) => Skin::From(m.clone()),
    };
    let shape = |f: &[f32; 4]| Shape { fingers: f[0], hand: f[1], wrist: f[2], forearm: f[3] };
    Recipe {
        from: a.from.clone(),
        skip: a.skip.clone(),
        shape: shape(&a.factors),
        per_mesh: a.per_mesh.iter().map(|(m, f)| (m.clone(), shape(f))).collect(),
        per_mesh_cuff: a.per_mesh_cuff.clone(),
        skin: skin(&a.skin),
        nails: a.nails,
        gloves: a.gloves.as_ref().map(|(c, from, to)| openxiii_assets::mod_arms::Gloves { colour: skin(c), from: *from, to: *to }),
        sleeve: a.sleeve.as_ref().map(|s| openxiii_assets::mod_arms::Sleeve {
            fabric: skin(&s.fabric),
            cuff: s.cuff,
            band: s.band,
            band_at: s.band_at,
            creases: s.creases,
            rings: s.rings.clone(),
            ring_colour: s.ring_colour,
            tab: s.tab,
            ribs: s.ribs,
            checks: s.checks,
            buttons: s.buttons,
            model: s.model.as_ref().map(|m| openxiii_assets::mod_arms::SleeveModel {
                from: m.from.clone(),
                thickness: m.thickness,
                room: m.room,
                lip: m.lip,
                seam: m.seam,
                painted: m.painted,
                folds: m.folds,
                fold_depth: m.fold_depth,
                fold_width: m.fold_width,
                fold_slant: m.fold_slant,
                taper: m.taper,
            }),
        }),
        hide_arm: a.hide_left.map(|l| if l { openxiii_assets::mod_arms::Side::Left } else { openxiii_assets::mod_arms::Side::Right }),
    }
}

/// The `[[map.<Map>.when]]` blocks of the mods as rules of the scripts' engine, each with its
/// description for the log ("when near Pam then say, give").
pub(crate) fn when_rules(edits: &[openxiii_mods::MapEdit]) -> Vec<(crate::mod_when::WhenRule, String)> {
    use crate::mod_when::{WhenAction, WhenArg, WhenEvent, WhenRule, WhenValue};
    use openxiii_mods::when::{Arg, Event, Value};
    fn value(v: &Value) -> WhenValue {
        match v {
            Value::Bool(b) => WhenValue::Bool(*b),
            Value::Int(i) => WhenValue::Int(*i),
            Value::Number(n) => WhenValue::Number(*n),
            Value::Text(t) => WhenValue::Text(t.clone()),
            Value::List(l) => WhenValue::List(l.iter().map(value).collect()),
            Value::Table(t) => WhenValue::Table(t.iter().map(|(k, x)| (k.clone(), value(x))).collect()),
        }
    }
    let mut out = Vec::new();
    for e in edits {
        let openxiii_mods::Edit::When(w) = &e.edit else { continue };
        let event = match &w.event {
            Event::Start => WhenEvent::Start,
            Event::Near { who, distance } => WhenEvent::Near { who: who.clone(), distance: *distance },
            Event::Enter { position, radius } => WhenEvent::Enter { position: *position, radius: *radius },
            Event::Sees { who } => WhenEvent::Sees { who: who.clone() },
            Event::Death { who } => WhenEvent::Death { who: who.clone() },
            Event::Damage { who } => WhenEvent::Damage { who: who.clone() },
            Event::Fire { who } => WhenEvent::Fire { who: who.clone() },
            Event::Pickup => WhenEvent::Pickup,
            Event::Event { name } => WhenEvent::Event { name: name.clone() },
            Event::Key { key } => WhenEvent::Key { key: key.clone() },
            Event::After { seconds } => WhenEvent::After { seconds: *seconds },
            Event::Every { seconds } => WhenEvent::Every { seconds: *seconds },
        };
        let actions = w
            .actions
            .iter()
            .map(|a| WhenAction {
                name: a.name.clone(),
                args: a
                    .args
                    .iter()
                    .map(|x| match x {
                        Arg::Value(v) => WhenArg::Value(value(v)),
                        Arg::Player => WhenArg::Player,
                        Arg::Actor => WhenArg::Actor,
                        Arg::Amount => WhenArg::Amount,
                        Arg::Class => WhenArg::Class,
                    })
                    .collect(),
            })
            .collect();
        let file = e.source.file.file_name().map_or_else(|| e.source.file.display().to_string(), |f| f.to_string_lossy().into_owned());
        let rule = WhenRule {
            mod_name: e.source.mod_name.clone(),
            map: e.map.clone(),
            source: format!("{}/{file} line {}", e.source.mod_name, e.source.line),
            event,
            actions,
            run: w.run.clone(),
            once: w.once,
            checkpoint: e.checkpoint,
            // The toml's start and timers count from when the player has control.
            control: matches!(w.event, Event::Start | Event::After { .. } | Event::Every { .. }),
        };
        out.push((rule, w.describe()));
    }
    out
}

/// Warnings for the mods' keys (key, where it is written) that the game's default settings
/// (`defaults`: the text of DefUser.ini) bind to a command of the game: the game keeps them.
pub(crate) fn game_bound_keys(defaults: &str, keys: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (key, source) in keys {
        let bound = defaults.lines().filter_map(|l| l.split_once('=')).find(|(k, v)| k.trim().eq_ignore_ascii_case(key) && !v.trim().is_empty());
        if let Some((_, command)) = bound {
            out.push(format!(
                "warning: key {key} ({source}) is bound by the game to \"{}\": the game keeps it; choose a free key ({})",
                command.trim(),
                openxiii_mods::behaviour::FREE_KEYS
            ));
        }
    }
    out
}

#[cfg(test)]
mod key_tests {
    /// A key of the game's default settings is reported; a free one is not.
    #[test]
    fn keys_bound_by_the_game() {
        let defaults = "[Engine.Input]\nF1=ShowScores | onrelease HideScores\nK=\nW=MoveForward\n";
        let keys = vec![("F1".to_string(), "a/mod.toml line 3".to_string()), ("K".to_string(), "a/mod.toml line 7".to_string()), ("w".to_string(), "b".to_string())];
        let out = super::game_bound_keys(defaults, &keys);
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out[0].contains("key F1 (a/mod.toml line 3) is bound by the game to \"ShowScores") && out[0].contains("free key (K, L"));
        assert!(out[1].contains("key w"));
    }

    /// Every key the shipped examples and the free-key list use is free in the game's real
    /// DefUser.ini (OXIII_GAME; skipped without it) and not kept by the game window.
    #[test]
    fn free_keys_are_free() {
        let free: Vec<String> = "K L J H U O P Insert".split(' ').map(String::from).chain((2..=9).map(|n| format!("NumPad{n}"))).collect();
        for k in &free {
            assert!(openxiii_mods::behaviour::reserved_key(k).is_none(), "{k}");
        }
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from) else { return };
        let defaults = std::fs::read_to_string(game.join("System").join("DefUser.ini")).unwrap();
        let keys: Vec<(String, String)> = free.iter().map(|k| (k.clone(), "test".into())).collect();
        assert!(super::game_bound_keys(&defaults, &keys).is_empty(), "{:?}", super::game_bound_keys(&defaults, &keys));
        // Negative control: F1 is the game's.
        assert_eq!(super::game_bound_keys(&defaults, &[("F1".into(), "t".into())]).len(), 1);
    }
}
