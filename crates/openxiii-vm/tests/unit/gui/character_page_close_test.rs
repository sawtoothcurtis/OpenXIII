//! Co-op character screen: leaving it closes every player's page (character_page.rs,
//! character_pages_close). A page left in the second player's view kept their gamepad in its
//! menu for the whole game (coop_pad_input reads coop_menu_open). OXIII_GAME = original game
//! folder; skipped when it is not set.

use std::path::PathBuf;

use crate::value::{ObjId, Value};
use crate::Vm;

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").is_dir().then_some(g)
}

/// A GUI controller of the game holding a character page (an XIIIWindowMainMenu) on its stack.
fn controller_with_page(vm: &mut Vm) -> (ObjId, ObjId) {
    let root_class = vm.class("XIIIRootWindow").expect("XIIIRootWindow");
    let root = vm.exec_new(None, None, 0, Some(root_class)).0.expect("new XIIIRootWindow");
    let page_class = vm.class("XIIIWindowMainMenu").expect("XIIIWindowMainMenu");
    let page = vm.exec_new(None, None, 0, Some(page_class)).0.expect("new XIIIWindowMainMenu");
    vm.set_prop(page, "Controller", Value::Object(Some(root)));
    vm.set_prop(root, "MenuStack", Value::Array(vec![Value::Object(Some(page))]));
    vm.set_prop(root, "ActivePage", Value::Object(Some(page)));
    (root, page)
}

fn stack(vm: &Vm, root: ObjId) -> usize {
    match vm.prop(root, "MenuStack") {
        Value::Array(a) => a.iter().filter(|p| p.as_object().is_some()).count(),
        _ => 0,
    }
}

/// The screen's held travel goes (everyone confirmed), or Back: both players' pages closed.
/// Control: a controller that holds no page of the screen keeps its own page.
#[test]
fn leaving_the_screen_closes_every_page() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    for back in [false, true] {
        let mut vm = Vm::new();
        vm.ipdrv.offline = true;
        vm.load_dir(&game.join("System")).unwrap();
        let (root1, page1) = controller_with_page(&mut vm);
        let (root2, page2) = controller_with_page(&mut vm);
        let (other, _) = controller_with_page(&mut vm);
        let request = crate::travel::TravelRequest {
            url: crate::travel::Url::parse(None, "Plage00", crate::travel::TRAVEL_ABSOLUTE),
            info: Vec::new(),
            engine: None,
            pending_join: false,
        };
        vm.improved.character_screen.on = true;
        vm.improved.character_screen.pages = vec![Some(page1), Some(page2)];
        vm.improved.character_screen.held = Some(crate::character_page::Held::Request { request, audio: Default::default() });
        if back {
            vm.improved.character_screen.back = true;
        } else {
            vm.improved.character_screen.go = true;
        }
        let out = vm.character_screen_travel(None);
        assert!(out.is_some(), "back {back}: a travel goes");
        assert_eq!((stack(&vm, root1), stack(&vm, root2)), (0, 0), "back {back}: every page closed");
        assert_eq!(stack(&vm, other), 1, "back {back}: another controller's page stays");
    }
}
