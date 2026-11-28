/// Improved build: a held direction on the page repeats as in the other menus (menu_repeat.rs):
/// one step, then the delay, then steady repeats; a release stops them; nothing repeats while
/// a remap waits nor from what was held when it ended; A and B never repeat.
#[test]
fn held_directions_repeat_like_the_other_menus() {
    use openxiii_vm::menu_repeat::{REPEAT_DELAY, REPEAT_PERIOD};
    const DT: f64 = 1.0 / 120.0;
    type State = dyn Fn(&GamepadPage, &GamepadConfig) -> String;
    /// Frames from `from` to `to` seconds with `word` / `stick` held: the times (from `from`)
    /// of the frames where `state` changed.
    fn hold(p: &mut GamepadPage, c: &mut GamepadConfig, word: u16, stick: (f32, f32), from: f64, to: f64, state: &State) -> Vec<f64> {
        let mut out = Vec::new();
        let mut t = from;
        while t < to - 1e-9 {
            let before = state(p, c);
            p.pad_at(word, stick, DT as f32, t, c, "int", &labels);
            if state(p, c) != before {
                out.push(t - from);
            }
            t += DT;
        }
        out
    }
    fn rest(p: &mut GamepadPage, c: &mut GamepadConfig, t: f64) {
        p.pad_at(0, (0.0, 0.0), DT as f32, t, c, "int", &labels);
    }
    let check = |steps: &[f64], what: &str| {
        assert!(steps.len() >= 3 && steps[0] == 0.0, "{what}: first step at once: {steps:?}");
        assert!((steps[1] - REPEAT_DELAY).abs() < 0.02, "{what}: then the delay: {steps:?}");
        for w in steps[1..].windows(2) {
            assert!((w[1] - w[0] - REPEAT_PERIOD).abs() < 0.02, "{what}: steady rate: {steps:?}");
        }
    };
    let focus: &State = &|p, _| p.focus.to_string();

    // Buttons list: d-pad down held 1 s.
    let (mut p, mut c) = page();
    let steps = hold(&mut p, &mut c, Button::Down.bit(), (0.0, 0.0), 0.0, 1.0, focus);
    check(&steps, "d-pad down");
    // 0, then 0.38 + k × 0.11 up to 1 s: 7 steps.
    assert_eq!(steps.len(), 7, "{steps:?}");
    // Released: no more steps, however long.
    assert!(hold(&mut p, &mut c, 0, (0.0, 0.0), 1.0, 2.0, focus).is_empty());
    // The stick held up the same way (with some sideways noise).
    check(&hold(&mut p, &mut c, 0, (0.3, -0.8), 2.0, 3.0, focus), "stick up");
    rest(&mut p, &mut c, 3.0);

    // Choice of the action by Right held in the "?": the edited button's action changes.
    p.focus_on(Item::Button(Button::X));
    p.key(Key::Enter, &mut c, "int", &labels);
    check(&hold(&mut p, &mut c, Button::Right.bit(), (0.0, 0.0), 4.0, 5.0, &|p, _| format!("{:?}", p.edit_choice())), "action");
    rest(&mut p, &mut c, 5.0);
    p.key(Key::Back, &mut c, "int", &labels);
    assert_eq!(c, GamepadConfig::default());

    // Advanced options: the look speed from 0 to 10 by the stick held right, one step a time.
    p.advanced = true;
    p.focus_on(Item::LookSpeed);
    c.players[0].look_speed = 0;
    let steps = hold(&mut p, &mut c, 0, (0.9, 0.1), 6.0, 7.5, &|_, c| c.players[0].look_speed.to_string());
    assert_eq!(c.players[0].look_speed, LOOK_SPEED_MAX);
    assert_eq!(steps.len(), usize::from(LOOK_SPEED_MAX), "{steps:?}");
    check(&steps, "look speed");
    rest(&mut p, &mut c, 7.5);
    p.advanced = false;

    // Left held on the buttons page walks across (wrapping at the edge): the press, the first
    // repeat and two more.
    p.focus_on(Item::Player);
    let before = c;
    let steps = hold(&mut p, &mut c, Button::Left.bit(), (0.0, 0.0), 8.0, 8.0 + REPEAT_DELAY + 2.5 * REPEAT_PERIOD, focus);
    assert_eq!(steps.len(), 4, "{steps:?}");
    rest(&mut p, &mut c, 9.0);
    assert_eq!(c, before, "moving across changed no setting");
    c = GamepadConfig::default();

    // A button edited: the d-pad down held neither answers it nor moves the focus; the X
    // button then answers it at once.
    p.focus_on(Item::Button(Button::A));
    p.key(Key::Enter, &mut c, "int", &labels);
    assert_eq!(p.waiting(), Some(Button::A));
    let steps = hold(&mut p, &mut c, Button::Down.bit(), (0.0, 0.0), 10.0, 12.0, focus);
    assert!(steps.is_empty(), "{steps:?}");
    assert_eq!((p.waiting(), c), (Some(Button::A), GamepadConfig::default()));
    rest(&mut p, &mut c, 12.0);
    hold(&mut p, &mut c, Button::X.bit(), (0.0, 0.0), 12.1, 12.5, focus);
    assert_eq!(p.waiting(), None, "X answered the edit");
    assert_eq!(c.players[0].action(Button::X), Action::Jump);
    assert_eq!(p.focused(), Item::Button(Button::X), "the focus follows the action");
    rest(&mut p, &mut c, 13.0);
    // The stick held right in the "?" scrolls the actions with the repeats, B cancels.
    let before = c;
    p.key(Key::Enter, &mut c, "int", &labels);
    check(&hold(&mut p, &mut c, 0, (0.8, 0.0), 13.1, 14.1, &|p, _| format!("{:?}", p.edit_choice())), "stick in the edit");
    rest(&mut p, &mut c, 14.2);
    hold(&mut p, &mut c, Button::B.bit(), (0.0, 0.0), 14.3, 14.4, focus);
    assert_eq!((p.waiting(), c), (None, before));
    rest(&mut p, &mut c, 14.5);

    // A held on Vibration (advanced page): one toggle only; B held: one step back only.
    p.advanced = true;
    p.focus_on(Item::Vibration);
    assert_eq!(hold(&mut p, &mut c, Button::A.bit(), (0.0, 0.0), 20.0, 21.5, &|_, c| c.players[0].vibration.to_string()).len(), 1);
    rest(&mut p, &mut c, 21.5);
    assert_eq!(hold(&mut p, &mut c, Button::B.bit(), (0.0, 0.0), 22.0, 23.5, &|p, _| format!("{} {}", p.advanced, p.open)).len(), 1);
    assert!(p.open && !p.advanced, "B held went back once, the page stays open");
}

/// Negative control: the same page without the repeater (as the faithful build) moves one
/// step only for a held direction, so the test above does see the repeats.
#[test]
fn without_the_repeater_a_held_direction_moves_once() {
    let (mut p, mut c) = page();
    let mut t = 0.0;
    while t < 1.0 {
        p.pad_at(Button::Down.bit(), (0.0, 0.0), 0.008, t, &mut c, "int", &labels);
        // Breaking the repeater: forget its state every frame.
        p.menu_navigation.repeat = Default::default();
        t += 0.008;
    }
    assert_eq!(p.focus, 1);
}
