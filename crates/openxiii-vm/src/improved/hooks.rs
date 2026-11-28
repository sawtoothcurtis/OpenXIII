//! Ordered improved script hooks installed around the common VM dispatch.
use crate::call_hooks::{Arguments, CallResult};
use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::Vm;
use std::ops::ControlFlow::{self, Break, Continue};

macro_rules! edition_hooks {
    ($($group:ident { $($hook:ident [$($name:literal),*];)* })*) => {
        #[derive(Clone, Copy)]
        pub(crate) enum Hook { $($($hook,)*)* }

        $(pub(crate) const $group: &[(Hook, &[&str])] = &[
            $((Hook::$hook, &[$($name),*]),)*
        ];)*
    };
}

edition_hooks! {
    BEFORE_VERSUS {
        BankKey ["Timer"];
    }
    BEFORE_COMIC_FRAME {
        MenuMouse ["FocusFirst"];
        HelpBarClick ["DisplayHelpBar"];
        WideMenu ["SetObjectives", "DisplayHelpBar"];
        HelpBarFit ["DisplayHelpBar"];
    }
    AFTER_COMIC_FRAME {
        HudSetup ["HUDSetup"];
        MenuBoxes ["Created", "Paint", "AfterPaint", "InternalOnClick", "InternalOnKeyEvent", "StartPressed"];
        Controls ["Created", "Paint", "BeforePaint", "InternalOnClick", "InternalOnKeyEvent"];
        PauseVideo ["Created", "Paint", "InternalOnClick"];
        VideoMenu ["BeforePaint", "Created", "Paint", "ShowWindow", "InternalOnKeyEvent"];
    }
}

impl Hook {
    pub(crate) fn run(self, vm: &mut Vm, f: FuncId, this: ObjId, mut args: Arguments) -> ControlFlow<CallResult, Arguments> {
        match self {
            Self::BankKey => {
                if vm.bank_key_timer(f, this) {
                    return Break(Ok((Value::Void, vec![None; args.len()])));
                }
            }
            Self::MenuMouse => {
                if vm.menu_mouse_focus_first(f, this, &args) {
                    let r = vm.call(f, this, args);
                    vm.menu_mouse_focus_first_end(this);
                    return Break(r);
                }
            }
            Self::HelpBarClick => {
                if vm.help_bar_click_begin(f, this, &args) {
                    let r = vm.call(f, this, args);
                    vm.help_bar_click_end();
                    return Break(r);
                }
            }
            Self::WideMenu => {
                if vm.improved.wide_menu.is_some() {
                    let c = args.first().and_then(|a| a.as_ref()).and_then(|a| a.as_object());
                    if let Some(call) = vm.wide_menu_call(f, this, c) {
                        let r = vm.call(f, this, args);
                        vm.wide_menu_call_end(call);
                        return Break(r);
                    }
                }
            }
            Self::HelpBarFit => {
                if let Some((c, org_x, org_y, clip_y)) = vm.help_bar_fit(f, this, &args) {
                    let r = vm.call(f, this, args);
                    vm.help_bar_fit_end(c, org_x, org_y, clip_y);
                    return Break(r);
                }
            }
            Self::HudSetup => {
                if crate::display_format::active(vm.engine.client.session_graphics) {
                    if let Some(c) = vm.hud_setup_wide_canvas(f, &args) {
                        let r = vm.call_inner(f, this, args);
                        vm.hud_setup_wide(this, c);
                        return Break(r);
                    }
                }
            }
            Self::MenuBoxes => {
                args = match vm.menu_boxes_call(f, this, args) {
                    Ok(r) => return Break(r),
                    Err(args) => args,
                };
            }
            Self::Controls => {
                args = match vm.controls_entry_call(f, this, args) {
                    Ok(r) => return Break(r),
                    Err(args) => args,
                };
            }
            Self::PauseVideo => {
                args = match vm.pause_video_call(f, this, args) {
                    Ok(r) => return Break(r),
                    Err(args) => args,
                };
            }
            Self::VideoMenu => {
                args = match vm.video_menu_call(f, this, args) {
                    Ok(r) => return Break(r),
                    Err(args) => args,
                };
            }
        }
        Continue(args)
    }
}

impl Vm {
    /// Improved build, solo and co-op: Banque01.STA_DonneCle.Timer, when the clerk takes the key
    /// back. The original puts the key away with XPC.NextWeapon + ChangedWeapon + PlayDown, made
    /// for a player holding the key or fists (the campaign gives no weapon before this point);
    /// with a real loaded weapon in hand (a chosen level and weapons) it switched to the next
    /// weapon by itself. Then only the rest is done: the event, the key removed, the state left.
    /// Returns true when it did the timer itself.
    pub(crate) fn bank_key_timer(&mut self, f: FuncId, this: ObjId) -> bool {
        let function = &self.functions[f];
        if !function.name.eq_ignore_ascii_case("Timer") || !function.state.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("STA_DonneCle")) {
            return false;
        }
        if !self.classes[function.owner].name.eq_ignore_ascii_case("Banque01") {
            return false;
        }
        let Some(pc) = self.prop(this, "XPC").as_object().filter(|&p| self.coop_object_alive(p)) else { return false };
        let Some(pawn) = self.prop(pc, "Pawn").as_object().filter(|&p| self.coop_object_alive(p)) else { return false };
        if !self.prop(pc, "bWeaponMode").as_bool() {
            return false;
        }
        let Some(weapon) = self.prop(pawn, "Weapon").as_object().filter(|&w| self.coop_object_alive(w)) else { return false };
        if self.is_a(weapon, "Fists") || !self.call_named(weapon, "HasAmmo", vec![]).is_ok_and(|v| v.as_bool()) {
            return false;
        }
        let event = self.prop(this, "Event");
        let _ = self.call_named(this, "TriggerEvent", vec![event, Value::Object(None), Value::Object(None)]);
        // CleBanque = Keys(XPC.Pawn.FindInventoryType(class'Keys')): the first Keys carried.
        let mut inv = self.prop(pawn, "Inventory").as_object();
        let mut guard = 0;
        while let Some(i) = inv.filter(|&i| self.coop_object_alive(i)) {
            if self.is_a(i, "Keys") {
                self.set_prop(this, "CleBanque", Value::Object(Some(i)));
                let _ = self.call_named(i, "Destroy", vec![]);
                break;
            }
            inv = self.prop(i, "Inventory").as_object();
            guard += 1;
            if guard > 256 {
                break;
            }
        }
        let _ = self.call_named(this, "GotoState", vec![Value::Name("None".into())]);
        self.log.push("Log: bank key taken back, weapon kept in hand".to_string());
        true
    }
}
