#[derive(Clone, Copy)]
struct MenuPageTouch;

/// Improved build, waiting behind the loading recap bubbles (main.rs recap_step): whether a finger
/// went down or a key or button was pressed since the last call. The fingers, keys and buttons
/// received meanwhile are not given to the game afterwards (as the PC drops the press that ends
/// the wait); the Edit panel's settings stay queued.
pub fn take_recap_press() -> bool {
    let Ok(mut f) = FILE.lock() else { return false };
    let press = f.iter().any(|e| matches!(e, Input::Finger(0, ..) | Input::Key(_, true) | Input::Button(_, true)));
    f.retain(|e| !matches!(e, Input::Finger(..) | Input::Key(..) | Input::Button(..) | Input::Wheel(_)));
    press
}

impl super::App {
    fn mods_touch_back(&mut self, down: bool) -> bool {
        if self.viewer.mods_page.open {
            if down {
                self.viewer.mods_page.key(super::mods_page::Key::Back);
            }
            return true;
        }
        false
    }

    fn begin_mods_touch(&mut self, id: i32, x: f32, y: f32) -> bool {
        if self.viewer.mods_page.touch(id as u64, super::choice_level::Finger::Pose, self.on_menu_page(x, y)) {
            self.touch_controls.finger = Some(Finger { id, x, y, mode: Mode::MenuPage(MenuPageTouch) });
            return true;
        }

        false
    }

    fn move_mods_touch(&mut self, id: i32, x: f32, y: f32, phase: super::choice_level::Finger) {
        let p = self.on_menu_page(x, y);
        self.viewer.mods_page.touch(id as u64, phase, p);
    }
}
