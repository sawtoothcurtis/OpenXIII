//! Cheats of the test builds (not in the original): Android's Edit panel and PC keys.
//! Only the "cheats" build of oxiii-view (`cheats` feature) calls them; the normal build
//! never touches these functions. Speed is `Vm::cheat_speed` (pawn_physics.rs).

use crate::value::{ObjId, Value};
use crate::Vm;

impl Vm {
    /// God mode: the player controller's bGodMode (like the game's God command).
    pub fn cheat_god(&mut self, controller: ObjId, on: bool) {
        if self.propr(controller, "bGodMode").as_bool() != on {
            self.set_prop(controller, "bGodMode", Value::Bool(on));
        }
    }

    /// Infinite ammo: each Ammunition in the pawn's inventory at MaxAmmo, ReloadCount and
    /// AltReloadCount of each weapon at its class's (full magazines).
    pub fn cheat_full_ammo(&mut self, pawn_value: ObjId) {
        let mut object_value = self.propr(pawn_value, "Inventory").as_object();
        let mut seen = 0;
        while let Some(o) = object_value.filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) {
            if self.is_a(o, "Ammunition") {
                let max = self.propr(o, "MaxAmmo").as_int();
                if self.propr(o, "AmmoAmount").as_int() < max {
                    self.set_prop(o, "AmmoAmount", Value::Int(max));
                }
            } else if self.is_a(o, "Weapon") {
                let cdo = self.objects[o].as_ref().map(|x| self.classes[x.class].cdo);
                for name_value in ["ReloadCount", "AltReloadCount"] {
                    let full = cdo.map_or(0, |c| self.propr(c, name_value).as_byte());
                    if self.propr(o, name_value).as_byte() < full {
                        self.set_prop(o, name_value, Value::Byte(full));
                    }
                }
            }
            object_value = self.propr(o, "Inventory").as_object();
            seen += 1;
            if seen > 256 {
                break;
            }
        }
    }
}

