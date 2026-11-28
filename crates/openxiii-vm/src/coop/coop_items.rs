//! Local co-op campaign: story items the map's script sets up on the first player's copy.
//!
//! The story's gifts go to every player (MapInfo.GiveSomething, coop.rs coop_share_gift), but
//! a map script that then sets the item it got back up (USA01.FirstFrame: the bomb's
//! BombInteraction, the spot where it can be put) only sets up the first player's one: the
//! other players' bombs could never be put in place. When a player uses such an item, the
//! references declared by the item's own class that are still empty on it are taken from the
//! same item of another player (the countdown and Palace01's microphone stay single items,
//! coop_shared.rs).

use crate::types::{FuncId, Type};
use crate::value::{ObjId, Value};
use crate::Vm;

impl Vm {
    /// Before Activate on a player's item, in co-op: its own empty references filled in from
    /// another player's item of the same class.
    pub(crate) fn coop_item_setup(&mut self, f: FuncId, this: ObjId) {
        if self.coop.players.len() < 2 || !self.functions[f].name.eq_ignore_ascii_case("Activate") || !self.is_a(this, "Inventory") {
            return;
        }
        let Some(class) = self.objects.get(this).and_then(|o| o.as_ref()).map(|o| o.class) else { return };
        let owner = self.prop(this, "Owner").as_object();
        let pawns: Vec<ObjId> = self.coop.players.iter().filter_map(|&pc| self.prop(pc, "Pawn").as_object()).filter(|&p| self.coop_object_alive(p)).collect();
        if !owner.is_some_and(|o| pawns.contains(&o)) {
            return;
        }
        // The references the item's own class declares (not those of Inventory and above).
        let first_own = self.classes[class].super_class.map_or(0, |s| self.classes[s].props.len());
        let own: Vec<String> = self.classes[class].props[first_own.min(self.classes[class].props.len())..]
            .iter()
            .filter(|p| matches!(p.ty, Type::Object(_)) && p.dim == 1)
            .map(|p| p.name.clone())
            .collect();
        let empty: Vec<String> = own.into_iter().filter(|n| self.prop(this, n).as_object().is_none()).collect();
        if empty.is_empty() {
            return;
        }
        for pawn in pawns.into_iter().filter(|&p| Some(p) != owner) {
            let mut item = self.prop(pawn, "Inventory").as_object();
            let mut guard = 0;
            while let Some(other) = item.filter(|&i| self.coop_object_alive(i)) {
                guard += 1;
                if guard > 256 {
                    break;
                }
                if self.objects[other].as_ref().is_some_and(|o| o.class == class) {
                    for name in &empty {
                        if let Some(v) = self.prop(other, name).as_object() {
                            self.set_prop(this, name, Value::Object(Some(v)));
                            self.log.push(format!("Log: Co-op: {}.{name} set up like {}", self.path_name(this), self.path_name(other)));
                        }
                    }
                    return;
                }
                item = self.prop(other, "Inventory").as_object();
            }
        }
    }
}
