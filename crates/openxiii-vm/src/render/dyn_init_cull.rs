//! Shared World/Replaced/Dynamic choice of the cull mode.
//! The determinant test of FDynamicActor::Init is not modelled yet.
pub fn cull_mode_history(two_sided: bool) -> u8 {
    if two_sided { 2 } else { 0 }
}

/// Exact order of the 24-bit x87 operations.
pub fn determinant3(m: &[f32; 16]) -> f32 {
    let m = m.map(f64::from);
    let a = mul24(add24(mul24(m[5],m[10]),-mul24(m[9],m[6])),m[0]);
    let b = mul24(add24(mul24(m[10],m[4]),-mul24(m[6],m[8])),m[1]);
    let c = mul24(add24(mul24(m[9],m[4]),-mul24(m[5],m[8])),m[2]);
    add24(add24(a,-b),c) as f32
}

// x87 precision is 24 bits, but its exponent does not become that of
// an f32 before FSTP. Products of three f32 inputs stay normal f64.
// TwoSum keeps the exact side of a tie when the double computation lands
// on a midpoint of the 24-bit rounding. No global FPU state is modified.
fn round24(p: f64, error: f64) -> f64 {
    if p == 0.0 || !p.is_finite() { return p; }
    const DROP: u64 = (1u64 << 29) - 1;
    const HALF: u64 = 1u64 << 28;
    let bits = p.to_bits();
    let low = bits & DROP;
    let up = if low == HALF && error != 0.0 {
        error.is_sign_negative() == p.is_sign_negative()
    } else { low > HALF || (low == HALF && ((bits >> 29) & 1) != 0) };
    f64::from_bits((bits & !DROP) + if up { 1u64 << 29 } else { 0 })
}
fn mul24(a: f64, b: f64) -> f64 { round24(a * b, 0.0) }
fn add24(a: f64, b: f64) -> f64 {
    let p = a + b;
    let bb = p - a;
    round24(p, (a - (p - bb)) + (b - bb))
}

/// Only extract: precision24 with x87 exponent,
/// CM_CCW for a negative ordered result; CM_CW for zero or unordered.
pub fn cull_mode_scene(view_determinant: f32, init_determinant: f32) -> u8 {
    // The sign of the exact product of two f32 is representable in f64,
    // even when a premature conversion to f32 would have given -0.
    u8::from((view_determinant as f64) * (init_determinant as f64) < 0.0)
}

/// Shared choice of the consumers: the TwoSided priority remains unchanged.
/// Without a draw produced by the VM, keeps the viewer's own choice (`legacy`).
pub fn cull_mode_draw(snapshot: Option<u8>, two_sided: bool, legacy: u8) -> u8 {
    if two_sided { 2 } else { snapshot.unwrap_or(legacy) }
}

/// Only non-pre-inverted Dynamic geometry. World/Replaced and the cartoon
/// paths keep their old choices until their representation is proven.
pub fn cull_mode_dynamic(snapshot: Option<u8>, dynamic: bool, two_sided: bool, legacy: u8) -> u8 {
    cull_mode_draw(if dynamic { snapshot } else { None },two_sided,legacy)
}
