use crate::{ObjId, Vm, value::Value};

impl Vm {
    /// AActor::PostEditChange, game path (GIsEditor = 0).
    pub(crate) fn actor_post_edit_change(&mut self, actor: ObjId) {
        // Native table, initialised; category 0 keeps the bits.
        const FLAGS: [[u8; 12]; 11] = [
            [0,0,0,0,0,0,0,0,0,0,0,0],
            [1,1,0,1,1,1,1,0,0,0,0,0],
            [1,1,0,1,1,1,1,1,0,0,0,0],
            [1,1,0,1,1,1,1,1,1,0,0,0],
            [0,1,0,0,0,1,1,0,0,0,1,0],
            [0,1,0,1,1,1,1,0,0,0,0,0],
            [0,1,0,1,1,1,1,1,0,0,0,0],
            [0,1,0,1,1,1,1,1,1,0,0,0],
            [0,1,1,1,1,1,1,0,0,0,1,1],
            [0,1,1,0,0,1,1,1,1,1,0,1],
            [0,1,1,0,0,1,1,1,0,0,0,1],
        ];
        let category = self.propr(actor, "ColCategory").as_byte();
        if let Some(flags) = category.checked_sub(1).and_then(|i| FLAGS.get(i as usize)) {
            for (name, bit) in crate::world::FLAGS_COL_CATEGORY.iter().zip(flags) {
                self.set_prop(actor, name, Value::Bool(*bit != 0));
            }
        }
        // GEngine->vtable+D4(61,0,0) is RET12 in UGameEngine/UEngine.
        // Outside the editor bLightChanged stays unchanged. StaticMeshInstance.Box.IsValid
        // is invalidated in the original; render_bounding_box always recomputes this
        // box in our architecture, without an instance box cache.
        self.update_leaves(actor);
    }
}
