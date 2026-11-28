//! Class tests of the tick (IsA of an engine class) stored as flags per class: same
//! answer as `is_a(a, "Name")`, computed once per class instead of a lookup in is_a's cache
//! (hashing and name comparison) on every call. The array is rebuilt when classes are
//! added and cleared together with the property slots (`clear_slot_cache`).

use crate::value::{ClassId, ObjId};
use crate::Vm;

macro_rules! flag_bits {
    ($($d:ident = $n:literal),* $(,)?) => {
        #[derive(Clone, Copy)]
        #[repr(u8)]
        pub(crate) enum D { $($d),* }
        pub(crate) const NAMES: &[&str] = &[$($n),*];
    };
}

flag_bits! {
    Actor = "Actor",
    Pawn = "Pawn",
    Controller = "Controller",
    PlayerController = "PlayerController",
    Emitter = "Emitter",
    Trail = "Trail",
    Decoration = "Decoration",
    Projector = "Projector",
    Mover = "Mover",
    Brush = "Brush",
    Projectile = "Projectile",
    BeamEmitter = "BeamEmitter",
    Volume = "Volume",
}

impl Vm {
    /// `is_a(a, flag name)`.
    #[inline]
    pub(crate) fn is(&self, a: ObjId, d: D) -> bool {
        let Some(o) = self.objects.get(a).and_then(|o| o.as_ref()) else { return false };
        self.class_is(o.class, d)
    }

    #[inline]
    pub(crate) fn class_is(&self, c: ClassId, d: D) -> bool {
        if let Ok(t) = self.flags_classes.try_borrow() {
            if t.len() == self.classes.len() {
                return t[c] >> d as u8 & 1 != 0;
            }
        }
        self.redo_flags_classes();
        self.flags_classes.borrow()[c] >> d as u8 & 1 != 0
    }

    fn redo_flags_classes(&self) {
        // Same name resolution as `is_a`: class_cache, otherwise class() (kept afterwards).
        let targets_list: Vec<Option<ClassId>> = NAMES
            .iter()
            .map(|n| {
                let cached = self.class_cache.borrow().get(*n).copied();
                cached.or_else(|| {
                    let c = self.class(n)?;
                    self.class_cache.borrow_mut().insert(n.to_string(), c);
                    Some(c)
                })
            })
            .collect();
        let t: Vec<u16> = (0..self.classes.len())
            .map(|c| targets_list.iter().enumerate().filter(|(_, p)| p.is_some_and(|p| self.is_child_of(c, p))).fold(0, |m, (k, _)| m | 1 << k))
            .collect();
        *self.flags_classes.borrow_mut() = t;
    }
}

#[cfg(test)]
mod tests {
}

