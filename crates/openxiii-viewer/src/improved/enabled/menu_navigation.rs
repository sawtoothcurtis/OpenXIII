#[derive(Default)]
pub(crate) struct MenuNavigation {
    stick: openxiii_vm::menu_repeat::Directions,
    repeat: openxiii_vm::menu_repeat::DirectionRepeat,
}

impl MenuNavigation {
    fn clock() -> f64 {
        openxiii_vm::menu_repeat::clock()
    }

    fn directions(&mut self, x: f32, y: f32) -> [bool; 4] {
        self.stick = openxiii_vm::menu_repeat::stick_directions(x, y, self.stick);
        self.stick
    }

    fn repeats(&mut self, held: [bool; 4], now: f64) -> [bool; 4] {
        self.repeat.update(held, now)
    }

    fn released(&mut self, now: f64) {
        self.stick = [false; 4];
        self.repeat.update([false; 4], now);
    }
}
