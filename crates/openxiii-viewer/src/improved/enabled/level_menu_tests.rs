/// Improved build: L (or gamepad Y) asks for the game's own Select level page; the list drawn
/// here never opens.
#[test]
fn improved_build_opens_the_game_page() {
    if !openxiii_vm::select_level::available() {
        eprintln!("skipped: {}", openxiii_vm::select_level::UNAVAILABLE);
        return;
    }
    let mut c = ChoiceLevel { menu: true, ..Default::default() };
    assert!(c.key_menu());
    assert!(c.want_page && !c.unlocked);
    let y = ButtonsMenu { open_flag: true, ..Default::default() };
    c.want_page = false;
    assert_eq!(c.pad(y, 0.016).0, None);
    assert!(c.want_page && !c.unlocked);
}
