impl GamepadPage {
    fn play_focus_sound(&mut self, vm: &mut Vm) {
        if self.focus_tick() {
            vm.play_menu_sound(openxiii_vm::menu_sounds::MenuSound::Tick);
        }
    }
}
