//! Fix that is not in the original game: hint rectangle on an already broken object.
//! CWndFocusTrigger.WaitForBeingSeen.Timer (XIII.u) shows a long hint (EventFinFocus, FocusDuration = -1) as
//! soon as the player looks at the target, without checking whether the object is still intact; only the
//! EventFinFocus event removes it (WaitEndFocus.Trigger). The Hual01b boxes (BreakableMover7/1/9/20 →
//! Dispatcher4/1/3, Break2 by the Dispatcher tagged Dispatcher2) only emit this event once, when broken: a box
//! broken before the check (assault rifle grenade, shot from afar, slow timer beyond TriggerDistance) leaves a
//! rectangle forever in the original. Here, when a long hint opens (GotoState WaitEndFocus) while its end
//! event has already been emitted by the breaking of a BreakableMover (directly, or through a Dispatcher it
//! triggered), it is closed at the end of the same ULevel::Tick by its own WaitEndFocus.Trigger, before any
//! drawing. Only in the improved build (feature `improved`, and in this crate's tests); the faithful build
//! always keeps the original behaviour.

use crate::value::{ObjId, Value};
use crate::NativeField as Field;
use crate::Vm;

impl Vm {
    /// After a successful GotoState: remembers the hint to close if its end event has passed.
    pub(crate) fn hint_open(&mut self, obj: ObjId, state: &str) {
        if !crate::improved::visual_effects::repair_focus_hints(self.engine.client.session_graphics) {
            return;
        }
        if !state.eq_ignore_ascii_case("WaitEndFocus") {
            return;
        }
        if self.objects.get(obj).and_then(|o| o.as_ref()).is_none() || !self.is_a(obj, "CWndFocusTrigger") {
            return;
        }
        let end_value = self.propr(obj, "EventFinFocus").as_name();
        if end_value.is_empty() || end_value.eq_ignore_ascii_case("None") || !self.end_broken_already(&end_value) {
            return;
        }
        if !self.indices_a_close.contains(&obj) {
            self.indices_a_close.push(obj);
        }
    }

    /// Was event `end_value` emitted by a broken BreakableMover (Breaked: TriggerEvent(Event)),
    /// directly or by one of the OutEvents of a Dispatcher that carries this Event's Tag?
    pub(crate) fn end_broken_already(&self, end_value: &str) -> bool {
        let broken_value: Vec<String> = self
            .actors
            .iter()
            .copied()
            .filter(|&a| self.objects.get(a).and_then(|o| o.as_ref()).is_some() && self.is_a(a, "BreakableMover"))
            .filter(|&a| self.propr(a, "bBroken").as_bool())
            .map(|a| self.propr(a, "Event").as_name())
            .filter(|e| !e.is_empty() && !e.eq_ignore_ascii_case("None"))
            .collect();
        if broken_value.iter().any(|e| e.eq_ignore_ascii_case(end_value)) {
            return true;
        }
        self.actors.iter().copied().any(|d| {
            self.objects.get(d).and_then(|o| o.as_ref()).is_some()
                && self.is_a(d, "Dispatcher")
                && broken_value.iter().any(|e| e.eq_ignore_ascii_case(&self.propr(d, "Tag").as_name()))
                && match self.propr(d, "OutEvents") {
                    Value::Array(v) => v.iter().any(|n| n.as_name().eq_ignore_ascii_case(end_value)),
                    v => v.as_name().eq_ignore_ascii_case(end_value),
                }
        })
    }

    /// End of ULevel::Tick: WaitEndFocus.Trigger of the remembered hints (RemoveMe of the rectangle, then
    /// Destroy of the trigger), as when their end event arrives.
    pub(crate) fn close_indices_broken(&mut self) {
        for obj in std::mem::take(&mut self.indices_a_close) {
            let alive = self.objects.get(obj).and_then(|o| o.as_ref()).is_some_and(|o| o.state.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("WaitEndFocus")));
            if !alive || self.native_prop(obj, Field::DeleteMe).as_bool() {
                continue;
            }
            if let Some(f) = self.find_function(obj, "Trigger", false) {
                if let Err(e) = self.call(f, obj, vec![Some(Value::Object(None)), Some(Value::Object(None))]) {
                    self.warn(format!("hint of {}: {e}", self.obj_label(obj)));
                }
            }
        }
    }
}

