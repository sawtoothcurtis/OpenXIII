impl ChoiceLevel {
    fn request_level_page(&mut self) -> bool {
        false
    }

    fn update_level_page(&mut self, _vm: &mut Vm, _front: bool) {}
}

impl ChoiceLevel {
    fn hide_replaced_level_entry(&mut self) -> bool {
        false
    }
}
