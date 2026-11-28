#[derive(Clone, Copy)]
enum MenuPageTouch {}

impl super::App {
    fn mods_touch_back(&mut self, _down: bool) -> bool {
        false
    }

    fn begin_mods_touch(&mut self, _id: i32, _x: f32, _y: f32) -> bool {
        false
    }

    fn move_mods_touch(&mut self, _id: i32, _x: f32, _y: f32, _phase: super::choice_level::Finger) {}
}
