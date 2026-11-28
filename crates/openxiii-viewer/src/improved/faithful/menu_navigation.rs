#[derive(Default)]
pub(crate) struct MenuNavigation;

impl MenuNavigation {
    fn clock() -> f64 {
        0.0
    }

    fn directions(&mut self, x: f32, y: f32) -> [bool; 4] {
        [y < -0.5, y > 0.5, x < -0.5, x > 0.5]
    }

    fn repeats(&mut self, _held: [bool; 4], _now: f64) -> [bool; 4] {
        [false; 4]
    }

    fn released(&mut self, _now: f64) {}
}
