//! Co-op: the body of a player who holds an object stops drawing its weapon.
//!
//! A weapon put down starts the "draw" gesture on the body's arms (XIIIWeapon DownWeapon →
//! XIIIWeaponAttachment.ThirdPersonSwitchWeapon → XIIIPawn.PlaySwitchingWeapon, channel 14,
//! bChangingWeapon). XIIIPawn.AnimEnd plays it again for as long as bChangingWeapon is set, and
//! only a weapon that comes up clears it (XIIIWeapon Active → SetFiringMode). A player who
//! takes an object in hand after a weapon (the grapple, a medkit, the lockpick) therefore keeps
//! drawing, over and over, until a weapon is back. Nobody sees a player's body in the solo game;
//! in co-op the other player does, on the grapple for instance.
//!
//! Here, in co-op only: when the gesture ends on the body of a player who holds an object, the
//! weapon change is over, and the game's own AnimEnd then blends the arms back.

use crate::value::Value;
use crate::{ObjId, Vm};

impl Vm {
    /// Called before AnimEnd(`ch`) reaches `pawn`'s scripts.
    pub(crate) fn coop_item_gesture_end(&mut self, pawn: ObjId, ch: i32) {
        if ch != 14 {
            return;
        }
        let Some(pc) = self.prop(pawn, "Controller").as_object() else { return };
        if !self.coop.players.contains(&pc) {
            return;
        }
        if !self.prop(pawn, "bChangingWeapon").as_bool() || self.prop(pc, "bWeaponMode").as_bool() {
            return;
        }
        self.set_prop(pawn, "bChangingWeapon", Value::Bool(false));
    }
}

#[cfg(test)]
mod tests {
    use crate::value::Value;

}
