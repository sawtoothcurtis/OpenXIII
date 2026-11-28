//! Split screen: each player's gamepad follows that player's own menu only. The other players'
//! gamepads read `coop_menu_open` of their controller (coop.rs, coop_pad_input); the first
//! player's reads the same rule through `slot_menu_open` (the viewer's pad_keys.rs). A menu open
//! in one player's view never puts another player's gamepad in a menu. OXIII_GAME = original game
//! folder; skipped when it is not set.

use std::path::PathBuf;

use crate::value::{ObjId, Value};
use crate::Vm;

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").is_dir().then_some(g)
}

fn new_of(vm: &mut Vm, class: &str) -> ObjId {
    let c = vm.class(class).unwrap_or_else(|| panic!("{class}"));
    vm.exec_new(None, None, 0, Some(c)).0.unwrap_or_else(|| panic!("new {class}"))
}

/// A player: a controller whose viewport holds a GUI controller (its menus), menu stack empty.
fn player(vm: &mut Vm) -> (ObjId, ObjId) {
    let pc = new_of(vm, "XIIIPlayerController");
    let viewport = new_of(vm, "Player");
    let menus = new_of(vm, "XIIIRootWindow");
    vm.set_prop(viewport, "LocalInteractions", Value::Array(vec![Value::Object(Some(menus))]));
    vm.set_prop(pc, "Player", Value::Object(Some(viewport)));
    (pc, menus)
}

fn open_menu(vm: &mut Vm, menus: ObjId, open: bool) {
    let stack = if open { vec![Value::Object(Some(new_of(vm, "XIIIWindowMainMenu")))] } else { Vec::new() };
    vm.set_prop(menus, "MenuStack", Value::Array(stack));
}

/// Player 1's menu open: player 2's gamepad stays in game (look, aim, fire), and the reverse;
/// both open: both in their menus. Control: the menu really counts for its own player.
#[test]
fn a_players_menu_takes_only_that_players_gamepad() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.load_dir(&game.join("System")).unwrap();
    let (pc1, menus1) = player(&mut vm);
    let (pc2, menus2) = player(&mut vm);
    for (one, two) in [(false, false), (true, false), (false, true), (true, true)] {
        open_menu(&mut vm, menus1, one);
        open_menu(&mut vm, menus2, two);
        assert_eq!((vm.coop_menu_open(pc1), vm.coop_menu_open(pc2)), (one, two), "menus open: player 1 {one}, player 2 {two}");
    }
}
