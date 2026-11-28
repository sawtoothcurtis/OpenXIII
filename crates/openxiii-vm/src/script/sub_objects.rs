//! Instanced sub-objects of an object constructed in game (UObject::InitProperties).

use crate::types::{Object, Type};
use crate::{ObjId, Value, Vm};

/// CPF 0x400000: object property whose value is copied for each instance (GUIMultiComponent.
/// Controls, Emitter.Emitters…), UObjectProperty::CopyCompleteValue.
const CPF_INSTANTIATES: u32 = 0x0040_0000;
/// CPF_Transient: taken from the class's default values in a copy.
const CPF_TRANSIENT: u32 = 0x2000;

impl Vm {
    /// Constructed object (StaticAllocateObject without RF_NeedLoad): InitProperties receives the object
    /// as SubobjectRoot when there is none, and the ConstructorLink chain
    /// calls CopyCompleteValue. For a CPF 0x400000 object property, each non-null value
    /// becomes StaticConstructObject(its class, Outer of the root, NAME_None, 0, template = itself,
    /// root), itself constructed the same way. Without this copy, the
    /// controls of a page (lblQuestion of XIIIMsgBox…) were those of the default values,
    /// shared by all pages; the page set MenuOwner on them and stayed referenced. The
    /// ParticleEmitters of an Emitter have their own copy (particles.rs, construct_emitters).
    pub(crate) fn instantiate_sub_objects(&mut self, id: ObjId, root_value: ObjId) {
        let Some(class) = self.objects[id].as_ref().map(|o| o.class) else { return };
        if self.class("Emitter").is_some_and(|e| self.is_child_of(class, e)) {
            return;
        }
        let slots: Vec<usize> = self.classes[class]
            .props
            .iter()
            .enumerate()
            // Array: UArrayProperty::CopyCompleteValue passes each element to the Inner, whose
            // CopyCompleteValue looks at its own flags (Controls: 0x400000; Skins: 0).
            .filter(|(_, p)| match &p.ty {
                Type::Object(_) => p.flags & CPF_INSTANTIATES != 0,
                Type::Array(t) => matches!(**t, Type::Object(_)) && p.inner_flags & CPF_INSTANTIATES != 0,
                _ => false,
            })
            .map(|(i, _)| i)
            .collect();
        for slot in slots {
            let Some(v) = self.objects[id].as_ref().map(|o| o.props[slot].clone()) else { return };
            let v = match v {
                Value::Array(items) => Value::Array(items.into_iter().map(|x| self.copy_value(x, root_value)).collect()),
                x => self.copy_value(x, root_value),
            };
            if let Some(o) = self.objects[id].as_mut() {
                o.props[slot] = v;
            }
        }
    }

    fn copy_value(&mut self, v: Value, root_value: ObjId) -> Value {
        let template = match &v {
            Value::Asset(path) => self.materialize_subobject(&path.clone()),
            Value::Object(Some(o)) => Some(*o),
            _ => None,
        }
        .filter(|&t| self.objects.get(t).is_some_and(|o| o.is_some()));
        match template {
            Some(t) => Value::Object(Some(self.copy_sub_object(t, root_value))),
            None => v,
        }
    }

    /// StaticConstructObject(template's class, Outer of the root, NAME_None, 0, template, root):
    /// template's properties, transients taken from the class, unique name in the Outer.
    pub(crate) fn copy_sub_object(&mut self, t: ObjId, root_value: ObjId) -> ObjId {
        let (class, mut props) = self.objects[t].as_ref().map(|o| (o.class, o.props.clone())).unwrap();
        let cdo = self.classes[class].cdo;
        for (i, p) in self.classes[class].props.iter().enumerate() {
            if p.flags & CPF_TRANSIENT != 0 {
                if let Some(d) = self.objects[cdo].as_ref().and_then(|o| o.props.get(i)) {
                    props[i] = d.clone();
                }
            }
        }
        let outer = self.objects[root_value].as_ref().map(|o| o.outer.clone()).unwrap_or_default();
        let outer_prop = self.prop(root_value, "Outer");
        let name = self.make_unique_name(class, &outer);
        self.object_paths.insert(format!("{outer}.{name}").to_lowercase());
        self.add_name(&name);
        self.objects.push(Some(Object { class, props, state: None, state_class: None, name, outer }));
        let id = self.objects.len() - 1;
        self.set_intrinsics(id);
        if self.prop_slot(id, "Outer").is_ok() {
            self.set_prop(id, "Outer", outer_prop);
        }
        self.instantiate_sub_objects(id, root_value);
        id
    }
}
