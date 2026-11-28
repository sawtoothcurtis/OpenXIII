//! Mods build only: changes to the actors of a map, on the real Plage01 (OXIII_GAME = original
//! game folder; skipped when it is not set).

use std::path::{Path, PathBuf};

use openxiii_mods::placement::{ActorValue, Edit, MapEdit, Source};
use openxiii_mods::ModValue;

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn edit(map: &str, edit: Edit) -> MapEdit {
    MapEdit { map: map.into(), edit, source: Source { mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1 }, checkpoint: Default::default() }
}

/// Plage01 started with `edits`, and the player logged in: (vm, player controller).
fn plage01(game: &Path, edits: Vec<MapEdit>) -> (Vm, ObjId) {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_edits_here(edits);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let players = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap();
    let pc = players[0];
    (vm, pc)
}

fn name(vm: &Vm, a: ObjId) -> String {
    vm.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default()
}

fn class_path(vm: &Vm, a: ObjId) -> String {
    vm.objects[a].as_ref().map(|o| vm.class_path(o.class)).unwrap_or_default()
}

/// Live actors of the level: (name, class).
fn actor_list(vm: &Vm) -> Vec<(String, String)> {
    let mut v: Vec<_> = vm.actors.iter().filter(|&&a| vm.objects[a].is_some() && !vm.native_prop(a, Field::DeleteMe).as_bool()).map(|&a| (name(vm, a), class_path(vm, a))).collect();
    v.sort();
    v
}

fn of_class(vm: &Vm, class: &str) -> Vec<ObjId> {
    vm.actors.iter().copied().filter(|&a| vm.objects[a].is_some() && !vm.native_prop(a, Field::DeleteMe).as_bool() && class_path(vm, a).eq_ignore_ascii_case(class)).collect()
}

fn inventory(vm: &Vm, pawn: ObjId) -> Vec<(ObjId, String)> {
    let mut out = Vec::new();
    let mut item = vm.prop(pawn, "Inventory").as_object();
    while let Some(i) = item {
        out.push((i, class_path(vm, i)));
        item = vm.prop(i, "Inventory").as_object();
    }
    out
}

#[test]
fn map_changes_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    // Baseline, and the same with changes for another map only: identical levels.
    let (base, base_pc) = plage01(&game, Vec::new());
    let other = plage01(&game, vec![edit("Bateau01", Edit::RemoveName { name: "Light0".into() })]).0;
    assert_eq!(actor_list(&base), actor_list(&other));
    assert!(other.mod_messages.is_empty(), "{:?}", other.mod_messages);

    // Where the player stands, and two kinds of pickups placed by the map.
    let pawn = base.native_prop(base_pc, Field::Pawn).as_object().unwrap();
    let [px, py, pz] = base.native_value(pawn, Field::Location).as_vector();
    let ground = pz - base.native_prop(pawn, Field::CollisionHeight).as_float();
    let mut pickups: Vec<(String, String, [f32; 3])> = base
        .actors
        .iter()
        .filter(|&&a| base.is_a(a, "Pickup") && !base.propr(a, "bStatic").as_bool() && !base.propr(a, "bNoDelete").as_bool())
        .map(|&a| (name(&base, a), class_path(&base, a), base.native_value(a, Field::Location).as_vector()))
        .collect();
    pickups.sort_by(|a, b| a.0.cmp(&b.0));
    eprintln!("player at [{px}, {py}, {ground}], pickups: {pickups:?}");
    assert!(pickups.len() >= 2, "{pickups:?}");
    let (removed_name, _, _) = pickups[0].clone();
    let (near_name, near_class, near_at) = pickups[1].clone();
    let replaced_class = pickups.iter().map(|p| p.1.clone()).find(|c| *c != near_class && pickups.iter().all(|p| p.1 != *c || p.0 != removed_name)).unwrap_or_else(|| near_class.clone());

    let edits = vec![
        edit("Plage01", Edit::Add { class: "XIII.FusilSnipePick".into(), position: [px + 150.0, py, ground], yaw: Some(90.0), properties: vec![] }),
        edit(
            "plage01",
            Edit::Add {
                class: "XIDPawn.BaseSoldier".into(),
                position: [px + 500.0, py, ground],
                yaw: Some(180.0),
                properties: vec![ActorValue { property: "Health".into(), index: None, value: ModValue::Int(321) }],
            },
        ),
        edit("Plage01", Edit::RemoveName { name: removed_name.clone() }),
        edit("Plage01", Edit::RemoveNear { class: near_class.clone(), near: near_at, radius: 10.0 }),
        edit("Plage01", Edit::Replace { from: replaced_class.clone(), to: "XIII.FusilChassePick".into() }),
        edit("Plage01", Edit::StartWeapon { class: "XIII.Beretta".into() }),
        edit("Plage01", Edit::StartWeapon { class: "XIII.FusilChasse".into() }),
        edit("Plage01", Edit::StartAmmo { class: "XIII.c9mmAmmo".into(), amount: 50 }),
        // Problems: one warning each, never a stop.
        edit("Plage01", Edit::Add { class: "XIII.NoSuchPick".into(), position: [0.0, 0.0, 0.0], yaw: None, properties: vec![] }),
        edit("Plage01", Edit::RemoveName { name: "NoSuchActor7".into() }),
        edit("Plage01", Edit::Add { class: "XIII.FusilSnipePick".into(), position: [900000.0, 900000.0, 900000.0], yaw: None, properties: vec![] }),
    ];
    let replaced_before: Vec<[f32; 3]> = of_class(&base, &replaced_class).iter().map(|&a| base.native_value(a, Field::Location).as_vector()).collect();
    let (mut vm, pc) = plage01(&game, edits.clone());
    let first_start = actor_list(&vm);
    let messages = vm.mod_messages.join("\n");
    eprintln!("{messages}");

    // Added pickup, at its place.
    let snipers = of_class(&vm, "XIII.FusilSnipePick");
    assert_eq!(snipers.len(), 1, "{messages}");
    let at = vm.native_value(snipers[0], Field::Location).as_vector();
    assert!((at[0] - px - 150.0).abs() < 1.0 && (at[1] - py).abs() < 1.0, "{at:?}");
    assert_eq!(vm.native_value(snipers[0], Field::Rotation).as_rotator()[1], 16384);
    // Added soldier, with its own value and an AI controller.
    let added: Vec<ObjId> = of_class(&vm, "XIDPawn.BaseSoldier").into_iter().filter(|&a| vm.prop(a, "Health") == Value::Int(321)).collect();
    assert_eq!(added.len(), 1, "{messages}");
    let soldier = added[0];
    let controller = vm.native_prop(soldier, Field::Controller).as_object();
    assert!(controller.is_some(), "added soldier has no controller");
    // Removed actors are gone; replaced ones changed class and kept their place.
    let names: Vec<String> = actor_list(&vm).into_iter().map(|(n, _)| n).collect();
    assert!(!names.iter().any(|n| n.eq_ignore_ascii_case(&removed_name)), "{removed_name} still there");
    assert!(!names.iter().any(|n| n.eq_ignore_ascii_case(&near_name)), "{near_name} still there");
    assert!(of_class(&vm, &replaced_class).is_empty());
    let new_places: Vec<[f32; 3]> = of_class(&vm, "XIII.FusilChassePick").iter().map(|&a| vm.native_value(a, Field::Location).as_vector()).collect();
    for p in &replaced_before {
        assert!(new_places.iter().any(|q| (q[0] - p[0]).abs() < 1.0 && (q[1] - p[1]).abs() < 1.0), "no replacement at {p:?}: {new_places:?}");
    }
    // Start weapons and ammo.
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    let inv = inventory(&vm, pawn);
    let has = |c: &str| inv.iter().find(|(_, k)| k.eq_ignore_ascii_case(c)).map(|(i, _)| *i);
    assert!(has("XIII.Beretta").is_some() && has("XIII.FusilChasse").is_some(), "{inv:?}");
    assert_eq!(vm.prop(has("XIII.c9mmAmmo").unwrap(), "AmmoAmount"), Value::Int(50));
    // Warnings.
    assert!(messages.contains("unknown class XIII.NoSuchPick"), "{messages}");
    assert!(messages.contains("remove NoSuchActor7: no such actor"), "{messages}");
    assert!(messages.contains("add XIII.FusilSnipePick at [900000, 900000, 900000]: "), "{messages}");

    // modpos.
    let (ok, text) = vm.console_command_out(pc, "modpos");
    assert!(ok, "{text}");
    assert!(text.starts_with("# Plage01\nposition = [") && text.contains("\nyaw = "), "{text}");

    // The player walks onto the added sniper rifle and picks it up.
    let pick_at = vm.native_value(snipers[0], Field::Location).as_vector();
    let mut picked = false;
    for frame in 0..(20 * 30) {
        if frame % 30 == 0 {
            let mut args = [Value::vector(pick_at[0], pick_at[1], pick_at[2] + 40.0)];
            let _ = crate::natives::call(&mut vm, "SetLocation", pawn, &mut args, &[true]);
        }
        vm.tick(1.0 / 30.0);
        if inventory(&vm, pawn).iter().any(|(_, c)| c.eq_ignore_ascii_case("XIII.FusilSnipe")) {
            picked = true;
            break;
        }
    }
    let state = vm.objects[snipers[0]].as_ref().map(|o| format!("{:?}", o.state)).unwrap_or_else(|| "destroyed".into());
    assert!(picked, "not picked up: bCanPickupInventory={}, pickup state {state}, inventory {:?}", vm.prop(pawn, "bCanPickupInventory"), inventory(&vm, pawn));

    // The added soldier sees the player and goes for him.
    let c = controller.unwrap();
    let mut enemy = None;
    for _ in 0..(20 * 30) {
        vm.tick(1.0 / 30.0);
        enemy = vm.prop(c, "Enemy").as_object();
        if enemy.is_some() {
            break;
        }
    }
    assert_eq!(enemy, Some(pawn), "the added soldier never targeted the player");

    // Checkpoint reload: the level is loaded again from its map, the changes are made once more,
    // not twice: same actors as at the first start.
    let (again, _) = plage01(&game, edits);
    assert_eq!(actor_list(&again), first_start);
    assert_eq!(of_class(&again, "XIII.FusilSnipePick").len(), 1);
}
