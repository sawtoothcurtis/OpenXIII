//! Numeric copy of the placement observed at one point of the render.
//! The UnrealScript properties stay Values and remain modifiable.
use crate::{value::{ObjId, Value}, NativeField as Field, Vm};

#[derive(Clone, Copy, Debug)]
pub struct RenderPlacement {
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    pub scale: [f32; 3],
    pub pre_pivot: [f32; 3],
}

impl RenderPlacement {
    pub fn read(vm: &Vm, actor: ObjId) -> Self {
        Self::from_values([Field::Location, Field::Rotation, Field::DrawScale,
            Field::DrawScale3D, Field::PrePivot].map(|p| vm.native_prop(actor, p)))
    }

    pub fn from_values(p: [&Value; 5]) -> Self {
        let s = p[2].as_float();
        let s = if s == 0.0 { 1.0 } else { s };
        let s3 = match p[3] {
            v if v.is_struct() => v.as_vector(),
            _ => [1.0; 3],
        };
        let s3 = if s3 == [0.0; 3] { [1.0; 3] } else { s3 };
        Self {
            location: p[0].as_vector(), rotation: p[1].as_rotator(),
            scale: [s * s3[0], s * s3[1], s * s3[2]],
            pre_pivot: p[4].as_vector(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_keeps_numbers_and_reference_scale_defaults() {
        let mut values = [Value::vector(-0.0, 2.0, 3.0), Value::rotator(i32::MIN, 7, 8),
            Value::Float(2.0), Value::vector(0.0, -3.0, 4.0), Value::vector(5.0, 6.0, 7.0)];
        let saved = RenderPlacement::from_values(values.each_ref());
        values[0] = Value::vector(10.0, 11.0, 12.0);
        assert_eq!(saved.location[0].to_bits(), (-0.0f32).to_bits());
        assert_eq!(saved.rotation, [i32::MIN, 7, 8]);
        assert_eq!(saved.scale, [0.0, -6.0, 8.0]);
        assert_eq!(saved.pre_pivot, [5.0, 6.0, 7.0]);
        for empty in [Value::Void, Value::vector(-0.0, 0.0, 0.0),
            Value::Struct("mod.short".into(), vec![Value::Float(4.0)].into())] {
            values[2] = Value::Float(-0.0); values[3] = empty;
            assert_eq!(RenderPlacement::from_values(values.each_ref()).scale, [1.0; 3]);
        }
    }
}
