//! Unreal's FCoords (origin and three axes, 0x30 bytes) and the Engine.dll operations that
//! combine them, copied one by one: the skeleton (GetFrame) and skinning chain them as
//! is, without going through quaternions.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
pub type Vec3 = [f32; 3];

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Coords {
    pub origin: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub z: Vec3,
}

/// GMath.UnitCoords.
pub const UNIT: Coords = Coords { origin: [0.0; 3], x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0], z: [0.0, 0.0, 1.0] };

fn dot(v: Vec3, a: Vec3) -> f32 {
    v[0] * a[0] + v[1] * a[1] + v[2] * a[2]
}

/// FVector::TransformVectorBy (dot products with the three axes).
pub fn transform_vector(v: Vec3, c: &Coords) -> Vec3 {
    [dot(v, c.x), dot(v, c.y), dot(v, c.z)]
}

/// FVector::TransformPointBy.
pub fn transform_point(p: Vec3, c: &Coords) -> Vec3 {
    transform_vector([p[0] - c.origin[0], p[1] - c.origin[1], p[2] - c.origin[2]], c)
}

impl Coords {
    /// FCoords * FCoords.
    pub fn mul(&self, b: &Coords) -> Coords {
        Coords { origin: transform_point(self.origin, b), x: transform_vector(self.x, b), y: transform_vector(self.y, b), z: transform_vector(self.z, b) }
    }

    /// FCoords::Matrix (frame → matrix that transforms points into it).
    pub fn matrix(&self) -> Matrix {
        let (ox, oy, oz) = (-self.origin[0], -self.origin[1], -self.origin[2]);
        let w = |a: Vec3| oz * a[2] + oy * a[1] + ox * a[0];
        let (x, y, z) = (self.x, self.y, self.z);
        [x[0], y[0], z[0], 0.0, x[1], y[1], z[1], 0.0, x[2], y[2], z[2], 0.0, w(x), w(y), w(z), 1.0]
    }

    /// FCoords::Transpose.
    pub fn transpose(&self) -> Coords {
        let o = transform_vector(self.origin, self);
        Coords {
            origin: [-o[0], -o[1], -o[2]],
            x: [self.x[0], self.y[0], self.z[0]],
            y: [self.x[1], self.y[1], self.z[1]],
            z: [self.x[2], self.y[2], self.z[2]],
        }
    }

    /// Bone `self` (local frame) composed with its parent's frame.
    pub fn compose_bone(&self, parent: &Coords) -> Coords {
        let o = transform_vector(self.origin, parent);
        Coords {
            origin: [o[0] + parent.origin[0], o[1] + parent.origin[1], o[2] + parent.origin[2]],
            x: transform_vector(parent.x, self),
            y: transform_vector(parent.y, self),
            z: transform_vector(parent.z, self),
        }
    }

    /// Applied to the axes: scalar triple product X · (Y × Z).
    fn determinant(&self) -> f32 {
        let (a, b, c) = (self.x, self.y, self.z);
        (b[0] * c[1] - c[0] * b[1]) * a[2] + (b[2] * c[0] - c[2] * b[0]) * a[1] + (c[2] * b[1] - b[2] * c[1]) * a[0]
    }

    /// Axes of the inverse (adjugate / determinant), shared.
    fn inverse_axes(&self, inv: f32) -> (Vec3, Vec3, Vec3) {
        let c = self;
        (
            [(c.y[1] * c.z[2] - c.z[1] * c.y[2]) * inv, (c.z[1] * c.x[2] - c.z[2] * c.x[1]) * inv, (c.y[2] * c.x[1] - c.y[1] * c.x[2]) * inv],
            [(c.z[0] * c.y[2] - c.y[0] * c.z[2]) * inv, (c.x[0] * c.z[2] - c.z[0] * c.x[2]) * inv, (c.y[0] * c.x[2] - c.x[0] * c.y[2]) * inv],
            [(c.z[1] * c.y[0] - c.y[1] * c.z[0]) * inv, (c.z[0] * c.x[1] - c.z[1] * c.x[0]) * inv, (c.x[0] * c.y[1] - c.y[0] * c.x[1]) * inv],
        )
    }

    /// FCoords::Inverse.
    pub fn inverse(&self) -> Coords {
        let inv = 1.0 / self.determinant();
        let (x, y, z) = self.inverse_axes(inv);
        let mut r = Coords { origin: [0.0; 3], x, y, z };
        r.origin = transform_vector([-self.origin[0], -self.origin[1], -self.origin[2]], &r);
        r
    }

    /// Inverse of the axes (zero determinant taken as 1), origin −(axes · origin).
    pub fn pivot_inverse(&self) -> Coords {
        let mut det = self.determinant();
        if det == 0.0 {
            det = 1.0;
        }
        let (x, y, z) = self.inverse_axes(1.0 / det);
        let o = transform_vector(self.origin, self);
        Coords { origin: [-o[0], -o[1], -o[2]], x, y, z }
    }
}

/// Entry `i` of GMath's sine table (TrigFLOAT, 16384 entries over a turn of 65536
/// units), filled by FGlobalMath::FGlobalMath: fild i, fmul by the float
/// 2π/16384 (0x39C90FDB, exact product in double), appSin, fstp to single. The single-precision
/// step matters: it shifts the angle of entry 16295 by 1.7e-7 rad.
pub fn gmath_table(i: usize) -> f32 {
    (i as f64 * f32::from_bits(0x39C9_0FDB) as f64).dsin() as f32
}

fn trig(angle: i32) -> f32 {
    static TABLE: std::sync::OnceLock<Box<[f32]>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| (0..=0x4000).map(gmath_table).collect());
    table[((angle >> 2) & 0x3fff) as usize]
}

/// GMath.SinTab[(angle >> 2) & 0x3FFF] (sine of an angle in 65536 units per turn).
pub fn gmath_sin(angle: i32) -> f32 {
    trig(angle)
}

#[cfg(test)]
#[test]
fn gmath_lookup_preserves_every_angle() {
    for angle in -32768i32..32768 {
        let expected = gmath_table(((angle >> 2) & 0x3fff) as usize);
        assert_eq!(gmath_sin(angle).to_bits(), expected.to_bits());
    }
}

/// FVector::RotateAngleAxis: `v` rotated by `angle` (Unreal units) around `axis`.
/// OMC · (Y·X), then each component as ((diagonal · V) + …) + …
/// in the order (Z), (Y), (X).
pub fn rotate_angle_axis(v: [f32; 3], angle: i32, axis: [f32; 3]) -> [f32; 3] {
    let s = trig(angle);
    let c = trig(angle.wrapping_add(0x4000));
    let (ax, ay, az) = (axis[0], axis[1], axis[2]);
    let omc = 1.0 - c;
    let xy = omc * (ay * ax);
    let yz = omc * (ay * az);
    let zx = omc * (az * ax);
    let (sx, sy, sz) = (s * ax, s * ay, s * az);
    [
        ((ax * ax * omc + c) * v[0] + (zx + sy) * v[2]) + (xy - sz) * v[1],
        ((ay * ay * omc + c) * v[1] + (yz - sx) * v[2]) + (xy + sz) * v[0],
        ((az * az * omc + c) * v[2] + (zx - sy) * v[0]) + (yz + sx) * v[1],
    ]
}

/// FCoords / FRotator (inverse rotation).
pub fn div_rotator(c: &Coords, r: [i32; 3]) -> Coords {
    let (pitch, yaw, roll) = (r[0], r[1], r[2]);
    let sr = trig(roll.wrapping_neg());
    let cr = trig(0x4000i32.wrapping_sub(roll));
    let sy = trig(yaw.wrapping_neg());
    let cy = trig(0x4000i32.wrapping_sub(yaw));
    let sp = trig(pitch.wrapping_neg());
    let cp = trig(0x4000i32.wrapping_sub(pitch));
    let l = Coords {
        origin: [0.0; 3],
        x: [cp * cy, sy * cr + sp * cy * sr, sp * cy * cr - sy * sr],
        y: [-(cp * sy), cy * cr - sp * sy * sr, -(cy * sr) - sp * sy * cr],
        z: [-sp, cp * sr, cp * cr],
    };
    c.mul(&l)
}

/// FCoords * FRotator.
pub fn mul_rotator(c: &Coords, r: [i32; 3]) -> Coords {
    let (pitch, yaw, roll) = (r[0], r[1], r[2]);
    let sr = trig(roll.wrapping_neg());
    let cr = trig(0x4000i32.wrapping_sub(roll));
    let sy = trig(yaw);
    let cy = trig(yaw.wrapping_add(0x4000));
    let sp = trig(pitch.wrapping_neg());
    let cp = trig(0x4000i32.wrapping_sub(pitch));
    let l = Coords {
        origin: [0.0; 3],
        x: [cp * cy, cp * sy, -sp],
        y: [sp * sr * cy - sy * cr, cy * cr + sp * sr * sy, cp * sr],
        z: [sy * sr + sp * cr * cy, sp * cr * sy - cy * sr, cp * cr],
    };
    c.mul(&l)
}

/// Unreal's FMatrix: 16 floats stored by rows (X, Y, Z, then W = translation), applied to
/// a row vector. Operations follow the exact addition order of the machine code.
pub type Matrix = [f32; 16];

/// Sine and cosine of a rotator as in the engine's matrices: sin(−roll), cos(roll),
/// sin(yaw), cos(yaw), sin(−pitch), cos(pitch) (GMath table).
fn rotator_terms(r: [i32; 3]) -> [f32; 6] {
    let (pitch, yaw, roll) = (r[0], r[1], r[2]);
    [
        trig(roll.wrapping_neg()),
        trig(0x4000i32.wrapping_sub(roll)),
        trig(yaw),
        trig(yaw.wrapping_add(0x4000)),
        trig(pitch.wrapping_neg()),
        trig(0x4000i32.wrapping_sub(pitch)),
    ]
}

/// FRotationTranslationMatrix (rotation then translation `t`).
pub fn rotation_translation_matrix(r: [i32; 3], t: Vec3) -> Matrix {
    let [f1, f2, f3, f4, f5, f6] = rotator_terms(r);
    [
        f6 * f4,
        f6 * f3,
        -f5,
        0.0,
        f5 * f1 * f4 - f3 * f2,
        f5 * f1 * f3 + f4 * f2,
        f6 * f1,
        0.0,
        f5 * f2 * f4 + f3 * f1,
        f5 * f2 * f3 - f4 * f1,
        f6 * f2,
        0.0,
        t[0],
        t[1],
        t[2],
        1.0,
    ]
}

/// Per-axis scale then rotation (no translation), a mesh's RotOrigin.
pub fn scale_rotation_matrix(r: [i32; 3], s: Vec3) -> Matrix {
    let [f1, f2, f3, f4, f5, f6] = rotator_terms(r);
    let f8 = f6 * s[0];
    [
        f8 * f4,
        f8 * f3,
        -(s[0] * f5),
        0.0,
        (f5 * f1 * f4 - f3 * f2) * s[1],
        (f4 * f2 + f5 * f1 * f3) * s[1],
        s[1] * f6 * f1,
        0.0,
        (f3 * f1 + f5 * f2 * f4) * s[2],
        (f5 * f2 * f3 - f4 * f1) * s[2],
        s[2] * f6 * f2,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

/// Product A × B; for each term, the order in which the four products are added.
pub fn matrix_mul(a: &Matrix, b: &Matrix) -> Matrix {
    const ORDER: [[usize; 4]; 16] = [
        [0, 2, 1, 3],
        [2, 0, 1, 3],
        [0, 2, 1, 3],
        [1, 3, 0, 2],
        [1, 2, 3, 0],
        [0, 1, 2, 3],
        [1, 2, 3, 0],
        [0, 3, 2, 1],
        [1, 2, 3, 0],
        [0, 1, 3, 2],
        [1, 3, 2, 0],
        [0, 3, 2, 1],
        [1, 2, 3, 0],
        [0, 1, 3, 2],
        [1, 3, 2, 0],
        [0, 3, 2, 1],
    ];
    let mut out = [0.0f32; 16];
    for (e, order) in ORDER.iter().enumerate() {
        let (i, j) = (e / 4, e % 4);
        let p = |k: usize| a[i * 4 + k] * b[k * 4 + j];
        out[e] = p(order[0]) + p(order[1]) + p(order[2]) + p(order[3]);
    }
    out
}

/// FMatrix::TransformFVector (point, with the translation).
pub fn transform_fvector(m: &Matrix, v: Vec3) -> Vec3 {
    [m[8] * v[2] + m[4] * v[1] + v[0] * m[0] + m[12], m[9] * v[2] + m[5] * v[1] + m[1] * v[0] + m[13], m[10] * v[2] + m[6] * v[1] + m[2] * v[0] + m[14]]
}

/// FMatrix::TransformNormal (without the translation).
pub fn transform_normal(m: &Matrix, v: Vec3) -> Vec3 {
    [m[8] * v[2] + m[4] * v[1] + v[0] * m[0], m[9] * v[2] + m[5] * v[1] + m[1] * v[0], m[10] * v[2] + m[6] * v[1] + m[2] * v[0]]
}

/// appRSqrt: 1 / sqrt.
pub fn rsqrt(x: f32) -> f32 {
    1.0 / x.sqrt()
}

/// FVector::Normalize, no effect below a squared length of 1e-8.
pub fn normalize(v: &mut Vec3) -> bool {
    let len2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if (len2 as f64) < 1e-8 {
        return false;
    }
    let s = rsqrt(len2);
    *v = [s * v[0], s * v[1], s * v[2]];
    true
}

/// FVector::SafeNormal (zero below a squared length of 1e-8).
pub fn safe_normal(v: Vec3) -> Vec3 {
    let len2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if (len2 as f64) < 1e-8 {
        return [0.0; 3];
    }
    let s = rsqrt(len2);
    [s * v[0], s * v[1], s * v[2]]
}

/// FBox(const FVector* Points, INT Count): zero, invalid box, then FBox +=
/// point after point: the first sets Min and Max; afterwards Min only moves if
/// Min <= V is false, Max only if Max >= V is false (so a NaN takes V). Returns Min, Max, IsValid.
pub fn box_from_points(points: &[Vec3]) -> (Vec3, Vec3, bool) {
    let (mut min, mut max, mut valid) = ([0.0f32; 3], [0.0f32; 3], false);
    for p in points {
        if !valid {
            (min, max, valid) = (*p, *p, true);
            continue;
        }
        for i in 0..3 {
            if !(min[i] <= p[i]) {
                min[i] = p[i];
            }
            if !(max[i] >= p[i]) {
                max[i] = p[i];
            }
        }
    }
    (min, max, valid)
}

/// FSphere(const FVector* Pts, INT Count): all zeros with no point; otherwise
/// centre at the middle of the points' box (box_from_points), W = the largest squared distance
/// (z² + y² + x², kept if strictly larger), radius sqrt(W) × 1.001 (double,
/// equal to the float 1.001; in 24-bit precision, a single rounding as in f32).
pub fn sphere_from_points(points: &[Vec3]) -> [f32; 4] {
    if points.is_empty() {
        return [0.0; 4];
    }
    let (min, max, _) = box_from_points(points);
    let c = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5];
    let mut w = 0.0f32;
    for p in points {
        let (dx, dy, dz) = (c[0] - p[0], c[1] - p[1], c[2] - p[2]);
        let d2 = (dz * dz + dy * dy) + dx * dx;
        if d2 > w {
            w = d2;
        }
    }
    [c[0], c[1], c[2], w.sqrt() * f32::from_bits(0x3f8020c5)]
}

/// + copy from GetFrame: frame of a bone from its quaternion and position (the
/// axes are the columns of the quaternion's matrix).
pub fn from_quat(q: [f32; 4], p: Vec3) -> Coords {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let xx = x2 * x;
    let xw = x2 * w;
    let m0 = 1.0 - (z2 * z + y2 * y);
    let m4 = y2 * x - z2 * w;
    let m8 = y2 * w + z2 * x;
    let m1 = z2 * w + y2 * x;
    let m5 = 1.0 - (z2 * z + xx);
    let m9 = z2 * y - xw;
    let m2 = z2 * x - y2 * w;
    let m6 = xw + z2 * y;
    let m10 = 1.0 - (y2 * y + xx);
    Coords { origin: p, x: [m0, m4, m8], y: [m1, m5, m9], z: [m2, m6, m10] }
}

