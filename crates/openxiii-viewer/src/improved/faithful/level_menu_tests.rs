#[test]
fn touches_list() {
    if !openxiii_vm::select_level::available() {
        eprintln!("skipped: {}", openxiii_vm::select_level::UNAVAILABLE);
        return;
    }
    let mut c = ChoiceLevel { menu: true, ..Default::default() };
    c.set.visible = 10;
    assert!(c.key_menu());
    assert!(c.unlocked);
    assert_eq!(c.key_col(Key::LevelNext), None);
    assert_eq!(c.key_col(Key::Low), None);
    assert_eq!(c.key_col(Key::Validate).as_deref(), Some("Plage01#teleporte_apres_lit"));
    assert!(!c.unlocked);
    c.open_flag();
    c.key_col(Key::EndEntry);
    assert_eq!(c.high[LEVELS_P], c.lines_n() - 10);
    c.key_col(Key::LevelPrevious);
    assert_eq!(lines_list()[c.choice[LEVELS_P]].url(), "SSH102b");
    c.key_col(Key::Return);
    assert!(!c.unlocked);
}

#[test]
fn page_of_weapons() {
    if !openxiii_vm::select_level::available() {
        eprintln!("skipped: {}", openxiii_vm::select_level::UNAVAILABLE);
        return;
    }
    let mut c = ChoiceLevel { menu: true, ..Default::default() };
    c.set.visible = 10;
    assert_eq!(c.classes_weapons(), ["XIII.Beretta", "XIII.c9mmAmmo", "XIII.m16", "XIII.M16Ammo", "XIII.M16GrenadAmmo"]);
    c.key_menu();
    c.key_col(Key::Weapons);
    assert_eq!(c.page, WEAPONS_P);
    // "No weapon", then check the shotgun (7th weapon): both its ammo types follow.
    c.key_col(Key::Low);
    assert_eq!(c.key_col(Key::Validate), None);
    assert!(c.classes_weapons().is_empty());
    for _ in 0..7 {
        c.key_col(Key::Low);
    }
    c.key_col(Key::Validate);
    assert_eq!(c.classes_weapons(), ["XIII.FusilPompe", "XIII.PompeAmmo", "XIII.ShotGunAltAmmo"]);
    // All: buckshot is only given once for the two shotguns.
    c.key_col(Key::Start);
    c.key_col(Key::Validate);
    let everything = c.classes_weapons();
    assert_eq!(everything.iter().filter(|&&x| x == "XIII.PompeAmmo").count(), 1);
    assert_eq!(everything.len(), 15 + 19 + 6); // + grappling hook, hook, medkit, vest, helmet
                                               // Back to the maps, still open; a second Back closes.
    c.key_col(Key::Return);
    assert_eq!(c.page, LEVELS_P);
    assert!(c.unlocked);
    c.key_col(Key::Return);
    assert!(!c.unlocked);
}

#[test]
fn gamepad_masked_until_at_release() {
    if !openxiii_vm::select_level::available() {
        eprintln!("skipped: {}", openxiii_vm::select_level::UNAVAILABLE);
        return;
    }
    let mut c = ChoiceLevel { menu: true, ..Default::default() };
    c.set.visible = 10;
    let y = ButtonsMenu { open_flag: true, ..Default::default() };
    assert_eq!(c.pad(y, 0.016), (None, true));
    assert!(c.unlocked);
    let low_value = ButtonsMenu { low_value: true, ..Default::default() };
    c.pad(low_value, 0.016);
    assert_eq!(c.choice[LEVELS_P], 1);
    c.pad(low_value, 0.3);
    assert_eq!(c.choice[LEVELS_P], 1);
    c.pad(low_value, 0.2);
    assert_eq!(c.choice[LEVELS_P], 2);
    let b = ButtonsMenu { return_value: true, ..Default::default() };
    assert_eq!(c.pad(b, 0.016), (None, true));
    assert!(!c.unlocked);
    assert_eq!(c.pad(ButtonsMenu::default(), 0.016), (None, false));
    let a = ButtonsMenu { validate: true, ..Default::default() };
    c.pad(y, 0.016);
    c.pad(ButtonsMenu::default(), 0.016);
    assert_eq!(c.pad(a, 0.016).0.as_deref(), Some("Plage01#teleporte_apres_lit"));
}

#[test]
fn touch_and_slide() {
    if !openxiii_vm::select_level::available() {
        eprintln!("skipped: {}", openxiii_vm::select_level::UNAVAILABLE);
        return;
    }
    let mut c = ChoiceLevel { menu: true, ..Default::default() };
    c.set = Set {
        input_value: [0.0, 400.0, 200.0, 480.0],
        return_value: [500.0, 0.0, 640.0, 40.0],
        weapons: [0.0, 40.0, 300.0, 50.0],
        list_value: [0.0, 50.0, 640.0, 450.0],
        line_h: 40.0,
        visible: 10,
    };
    assert!(!c.press(Some((300.0, 100.0))));
    assert!(c.press(Some((50.0, 450.0))));
    assert_eq!(c.released(Some((50.0, 450.0))), None);
    assert!(c.unlocked);
    // Dragging up by three lines: the list scrolls, nothing is started.
    assert!(c.press(Some((100.0, 300.0))));
    c.moves((100.0, 180.0));
    assert_eq!(c.released(Some((100.0, 180.0))), None);
    assert_eq!(c.high[LEVELS_P], 3);
    // Touching the weapons line opens their page; Back returns.
    assert!(c.press(Some((100.0, 45.0))));
    c.released(Some((100.0, 45.0)));
    assert_eq!(c.page, WEAPONS_P);
    assert!(c.press(Some((600.0, 20.0))));
    c.released(Some((600.0, 20.0)));
    assert_eq!(c.page, LEVELS_P);
    // Touching the second visible line: the map on line 4 starts.
    assert!(c.press(Some((100.0, 95.0))));
    assert_eq!(c.released(Some((100.0, 95.0))).as_deref(), Some("Banque01#Teleporter_bricole"));
    assert!(!c.unlocked);
}
