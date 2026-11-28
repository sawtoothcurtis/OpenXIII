impl ChoiceLevel {
    fn request_level_page(&mut self) -> bool {
        self.want_page = true;
        true
    }

    fn update_level_page(&mut self, vm: &mut Vm, front: bool) {
        if let Some(mask) = vm.take_select_level_weapons_changed() {
            self.weapons = mask;
        }
        vm.set_select_level_weapons(self.weapons);
        if std::mem::take(&mut self.want_page) && front {
            vm.open_select_level_page();
        }
    }
}

impl ChoiceLevel {
    fn hide_replaced_level_entry(&mut self) -> bool {
        if self.unlocked {
            return false;
        }
        self.set.input_value = [0.0; 4];
        true
    }
}
