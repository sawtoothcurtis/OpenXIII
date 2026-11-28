//! Native functions, called by their name (e.g. `Add_IntInt`, `Log`, `IsA`).
//!
//! Operators follow Unreal's naming scheme: `Operation_TypeType`,
//! `Operation_PreType` (prefix) or `Operation_Type` (postfix). `out` parameters
//! are modified directly in `args`.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::{ObjId, Value};
use crate::{Result, Vm, VmError};

/// UObject::execGetAxes: FCoords(0) / R (GMath's sine table).
/// LoadFlags of StaticLoadObject (UnObjBas.h).
pub(crate) const LOAD_NO_FAIL: u32 = 0x1;
pub(crate) const LOAD_NO_WARN: u32 = 0x2;
pub(crate) const LOAD_THROW: u32 = 0x8;
pub(crate) const LOAD_QUIET: u32 = 0x2000;

/// UObject::SafeLoadError: formatted message (appGetVarArgs), then
/// in order GLog (NAME_Warning) unless LOAD_Quiet, appThrowf(Error) if LOAD_Throw, GError if LOAD_NoFail, GWarn
/// unless LOAD_NoWarn. Returns (device, text) for each output.
pub(crate) fn safe_load_error(flags: u32, error: &str, fmt: &str, args: &[&str]) -> Vec<(&'static str, String)> {
    let message = crate::value::printf_s(fmt, args);
    let mut out = Vec::new();
    if flags & LOAD_QUIET == 0 {
        out.push(("GLog", message.clone()));
    }
    if flags & LOAD_THROW != 0 {
        out.push(("Throw", error.to_string()));
    }
    if flags & LOAD_NO_FAIL != 0 {
        out.push(("GError", message.clone()));
    }
    if flags & LOAD_NO_WARN == 0 {
        out.push(("GWarn", message));
    }
    out
}

fn rot_axes(r: [i32; 3]) -> [[f32; 3]; 3] {
    crate::ai_natives::axes(r)
}

fn vec(v: [f32; 3]) -> Value {
    Value::vector(v[0], v[1], v[2])
}

/// `*dest = vec(v)`, rewriting the fields of a vector already there (without a new list).
fn vec_in(dest: &mut Value, v: [f32; 3]) {
    match dest {
        Value::Vector(a) => *a = v,
        Value::Struct(n, f) if *n == crate::value::NameStruct::VECTOR && f.len() == 3 => {
            for (c, x) in f.iter_mut().zip(v) {
                *c = Value::Float(x);
            }
        }
        _ => *dest = vec(v),
    }
}

fn rot(r: [i32; 3]) -> Value {
    Value::rotator(r[0], r[1], r[2])
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// FVector | FVector of UObject::execDot_VectorVector: (Z + Y) + X.
fn core_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    core_dot_x87(a, b) as f32
}

/// (Bz·Az + By·Ay) + Bx·Ax on the x87 stack: each operation rounded to 24 bits, without
/// float overflow before storing (3.4e38 gives infinity).
fn core_dot_x87(a: [f32; 3], b: [f32; 3]) -> f64 {
    let m = |i: usize| x87_24(b[i] as f64 * a[i] as f64);
    x87_24(x87_24(m(2) + m(1)) + m(0))
}

/// (_ftol2): truncation on 64 bits, low 32 bits; out of range (infinity,
/// NaN) the undefined integer 0x8000000000000000, i.e. 0 (value::ftol; `as i64` saturated at
/// 0x7FFFFFFFFFFFFFFF, i.e. −1).
fn ftol(x: f32) -> i32 {
    crate::value::ftol(x) as i32
}

/// FILD of an integer then FMUL / FDIV by a float: exact operation rounded to 24 bits.
fn int_mul(i: i32, f: f32) -> f32 {
    (i as f64 * f as f64) as f32
}

fn text(vm: &Vm, v: &Value) -> String {
    match v {
        Value::Object(Some(o)) => vm.objects[*o].as_ref().map(|o| o.name.clone()).unwrap_or_default(),
        Value::Class(Some(c)) => vm.classes[*c].name.clone(),
        other => other.to_string(),
    }
}

impl Vm {
    /// AIAController::AllianceLevel (XIDPawn.dll): -1 for the player (XIII), 1 for
    /// everyone if LevelInfo.bNoEnemyAlliance, otherwise AllianceLevel (truncated by _ftol) of the entry of
    /// BaseS.InitialAlliances[0..4] whose name is the other pawn's Alliance; 0 if none, -1 without BaseS.
    pub fn alliance_level(&self, controller: ObjId, other: Option<ObjId>) -> i32 {
        // Mods build only: the team of a mod character towards the player (mod_behaviour.rs).
        #[cfg(feature = "mods")]
        if let Some(level) = self.mod_alliance(controller, other) {
            return level;
        }
        // Pointer comparison: None against a missing XIII also gives -1.
        if other == self.propr(controller, "XIII").as_object() {
            return -1;
        }
        if self.level.is_some_and(|l| self.propr(l, "bNoEnemyAlliance").as_bool()) {
            return 1;
        }
        let Some(soldier) = self.propr(controller, "BaseS").as_object() else {
            return -1;
        };
        let Some(Value::Name(other)) = other.map(|o| self.prop(o, "Alliance")) else {
            return 0;
        };
        if other.is_empty() {
            return 0;
        }
        if let Value::Array(items) = self.prop(soldier, "InitialAlliances") {
            for item in items.iter().take(4) {
                if let Value::Struct(_, fields) = item {
                    if matches!(fields.first(), Some(Value::Name(a)) if a.eq_ignore_ascii_case(&other)) {
                        // _ftol2: truncation on 64 bits, low 32 bits (0 out of range).
                        return crate::value::ftol(fields.get(1).map(|v| v.as_float()).unwrap_or(0.0)) as i32;
                    }
                }
            }
        }
        0
    }

    /// appRand: rand() from Microsoft's C library (0 to 32767). The
    /// seed starts from a fixed value (the game takes appCycles()), for stable screenshots.
    #[track_caller]
    pub(crate) fn app_rand(&mut self) -> i32 {
        let file = std::panic::Location::caller().file();
        // Core natives (FRand, VRand, RotRand…): Core's appFrand/appRand; the rest comes from a
        // native module (Engine, XIDPawn…).
        let kind_value = if file.ends_with("natives.rs") { crate::draw_kind::S } else { crate::draw_kind::N };
        self.app_rand_kind(kind_value)
    }

    /// appRand of execRand (Core 0x19844), which the tape tells apart from appFrand.
    #[track_caller]
    pub(crate) fn app_rand_script(&mut self) -> i32 {
        self.app_rand_kind(crate::draw_kind::R)
    }

    /// New rand() state returned by HXAudio (`key`: the caller, for the draw diagnostics).
    pub(crate) fn seed_hx(&mut self, new: u32, key: &str) {
        let _ = key;
        self.seed = new;
    }

    /// New rand() state set by a draw made outside app_rand (copy of the sequence: particles,
    /// HXAudio, loading animation); `kind_value` and `key` are for the draw diagnostics.
    pub(crate) fn seed_advanced(&mut self, new: u32, kind_value: u8, key: &str) {
        let _ = (kind_value, key);
        self.seed = new;
    }

    #[track_caller]
    fn app_rand_kind(&mut self, kind_value: u8) -> i32 {
        let wanted = self.rand_counts.is_some();
        let _ = kind_value;
        if wanted {
            let at = std::panic::Location::caller();
            let file = at.file().rsplit(['/', '\\']).next().unwrap_or("");
            // Rand / FRand / VRand…: the script function that calls the native, named like
            // Stack.Node (the state for a state function: "CineInit.Timer").
            let key = if file == "natives.rs" {
                let n = self.stack.len();
                let f = |i: usize| {
                    self.stack.get(i).map(|&f| {
                        let d = &self.functions[f];
                        format!("{}.{}", d.state.as_deref().unwrap_or(&self.classes[d.owner].name), d.name)
                    })
                };
                let state_value = || {
                    let o = self.this_stack.last().copied().or(self.state_code_obj);
                    format!("state code of {}", o.and_then(|o| self.objects.get(o)?.as_ref().map(|x| x.name.clone())).unwrap_or_default())
                };
                format!("{} < {}", f(n.wrapping_sub(1)).unwrap_or_default(), f(n.wrapping_sub(2)).unwrap_or_else(state_value))
            } else {
                format!("{file}:{}", at.line())
            };
            if let Some(counts) = self.rand_counts.as_mut() {
                *counts.entry(key).or_default() += 1;
            }
        }
        self.seed = self.seed.wrapping_mul(214_013).wrapping_add(2_531_011);
        ((self.seed >> 16) & 0x7fff) as i32
    }

    /// rand() state of the game thread (msvcr70's _tiddata->_holdrand, shared with HXAudio).
    pub fn rand_seed(&self) -> u32 {
        self.seed
    }

    /// Sets the rand() state of the game thread (seed recorded in the original game).
    pub fn set_rand_seed(&mut self, seed: u32) {
        self.seed = seed;
    }

    /// The game's launch (not the tests and tools, which keep the fixed seed): the game thread's
    /// rand() seeded from the clock as in the original. Core's appInit does
    /// srand(time(NULL)), and HXAudio's first CUUID does
    /// srand(rdtsc >> 32) again (`hx_srand_clock`). Returns whether the clock was
    /// used.
    pub fn seed_from_clock(&mut self) -> bool {
        // msvcr70's time() is a 32-bit time_t.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.seed = now as u32;
        self.hx_srand_clock = true;
        true
    }

    /// The seeds a lockstep session starts from (the host's, sent to the guest): the game
    /// thread's rand() state and the seed of HXAudio's first-CUUID srand.
    pub fn lockstep_seeds(&self) -> (u32, Option<u32>) {
        let hx = if self.hx_srand_clock { Some(openxiii_hx::rand::clock_srand_seed()) } else { self.hx_srand_given };
        (self.seed, hx)
    }

    /// See [`Vm::lockstep_seeds`]: this VM starts from them, its first-CUUID srand included.
    pub fn set_lockstep_seeds(&mut self, seed: u32, hx: Option<u32>) {
        self.seed = seed;
        self.hx_srand_clock = false;
        self.hx_srand_given = hx;
    }

    /// appFrand: rand() × (1 / 32767), from 0 to 1 inclusive.
    #[track_caller]
    pub(crate) fn rand(&mut self) -> f32 {
        self.app_rand() as f32 * 3.051_851e-5
    }
}

// Core.dll opcodes and operators (UObject::exec…) that the VM runs without citing them elsewhere,
// audited against the decompilation: interpreter (lib.rs,
// Kind::…), operators by name (binary, assign_op), conversions (lib.rs convert). Floats
// follow the asm (24-bit FPU: one f32 result per operation), A / F of vectors and rotators
// as A × (1 / F).
// Undefined, BoolVariable, Nothing, EndFunctionParms,
// Stop, Self, ByteConst, IntZero, IntOne,
// True, False, NoObject, IntConstByte,
// PrimitiveCast, VectorConst, HighNative0, HighNative1,
// HighNative2, HighNative3, HighNative4, HighNative5,
// HighNative6, HighNative7, HighNative8, HighNative9,
// HighNative10, HighNative11, HighNative12,
// HighNative13, HighNative14, HighNative15, Iterator,
// LocalVariable, InstanceVariable, DefaultVariable,
// DynArrayLength, NativeParm, LetBool, VirtualFunction,
// FinalFunction, GlobalFunction, DelegateFunction,
// DelegateProperty, LetDelegate, IntConst, FloatConst,
// StringConst, UnicodeStringConst, ObjectConst,
// NameConst, DynamicCast, ByteToInt, ByteToBool,
// ByteToFlo, ByteToString, IntToByte, IntToBool,
// IntToFlo, IntToString, BoolToByte, BoolToInt,
// BoolToFlo, BoolToString, FloatToBool, ObjectToBool,
// NameToBool, NameToString, Not_PreBool,
// EqualEqual_BoolBool, NotEqual_BoolBool, AndAnd_BoolBool,
// XorXor_BoolBool, OrOr_BoolBool, MultiplyEqual_ByteByte,
// AddEqual_ByteByte, SubtractEqual_ByteByte, AddAdd_PreByte,
// SubtractSubtract_PreByte, AddAdd_Byte, SubtractSubtract_Byte,
// Complement_PreInt, GreaterGreaterGreater_IntInt,
// Subtract_PreInt, Multiply_IntInt, Divide_IntInt,
// Add_IntInt, Subtract_IntInt, LessLess_IntInt,
// GreaterGreater_IntInt, Less_IntInt, Greater_IntInt,
// LessEqual_IntInt, GreaterEqual_IntInt, EqualEqual_IntInt,
// NotEqual_IntInt, And_IntInt, Xor_IntInt, Or_IntInt,
// AddEqual_IntInt, SubtractEqual_IntInt, AddAdd_PreInt,
// SubtractSubtract_PreInt, AddAdd_Int, SubtractSubtract_Int,
// Min, Max, Subtract_PreFlo, Multiply_FloatFlo,
// Add_FloatFlo, Subtract_FloatFlo, Less_FloatFlo,
// Greater_FloatFlo, LessEqual_FloatFlo,
// GreaterEqual_FloatFlo, EqualEqual_FloatFlo,
// NotEqual_FloatFlo, MultiplyEqual_FloatFlo,
// DivideEqual_FloatFlo, AddEqual_FloatFlo,
// SubtractEqual_FloatFlo, Cos, Tan, Loge,
// FRand, RotationConst, VectorToRotator,
// RotatorToVector, Subtract_PreVector, Multiply_VectorFlo,
// Multiply_FloatVector, Multiply_VectorVector, Add_VectorVector,
// Subtract_VectorVector, EqualEqual_VectorVector,
// NotEqual_VectorVector, Cross_VectorVector,
// MultiplyEqual_VectorFlo, MultiplyEqual_VectorVector,
// DivideEqual_VectorFlo, AddEqual_VectorVector,
// SubtractEqual_VectorVector, EqualEqual_RotatorRotator,
// NotEqual_RotatorRotator, Multiply_FloatRotator,
// MultiplyEqual_RotatorFlo, DivideEqual_RotatorFlo,
// Add_RotatorRotator, Subtract_RotatorRotator,
// AddEqual_RotatorRotator, SubtractEqual_RotatorRotator,
// OrthoRotation, Normalize, EatString,
// Less_StringString, Greater_StringString, LessEqual_StringString,
// GreaterEqual_StringString, EqualEqual_StringString,
// NotEqual_StringString, ComplementEqual_StringString, InStr,
// EqualEqual_NameName, NotEqual_NameName, EqualEqual_ObjectObject,
// NotEqual_ObjectObject, StructMember, Switch,
// MetaCast, Concat_StringString, At_StringString, Caps,
// Lowcaps, Case, StructCmpEq,
// StructCmpNe
/// Arithmetic or comparison operator on two values of the same type.
fn binary(op: &str, ty: &str, a: &Value, b: &Value) -> Option<Value> {
    let [ta, tb] = split_types(ty);
    binary_types(op, ta.unwrap_or(""), tb.unwrap_or(""), a, b)
}

fn binary_types(op: &str, ta: &str, tb: &str, a: &Value, b: &Value) -> Option<Value> {
    Some(match (ta, tb) {
        ("Int", "Int") | ("Byte", "Byte") => {
            let (x, y) = (a.as_int(), b.as_int());
            let v = match op {
                "Add" => x.wrapping_add(y),
                "Subtract" => x.wrapping_sub(y),
                "Multiply" => x.wrapping_mul(y),
                "Divide" => {
                    if y == 0 {
                        0
                    } else {
                        x.wrapping_div(y)
                    }
                }
                "LessLess" => x.wrapping_shl(y as u32),
                "GreaterGreater" => x.wrapping_shr(y as u32),
                "GreaterGreaterGreater" => ((x as u32).wrapping_shr(y as u32)) as i32,
                "And" => x & y,
                "Or" => x | y,
                "Xor" => x ^ y,
                "Less" => return Some(Value::Bool(x < y)),
                "Greater" => return Some(Value::Bool(x > y)),
                "LessEqual" => return Some(Value::Bool(x <= y)),
                "GreaterEqual" => return Some(Value::Bool(x >= y)),
                "EqualEqual" => return Some(Value::Bool(x == y)),
                "NotEqual" => return Some(Value::Bool(x != y)),
                _ => return None,
            };
            if ta == "Byte" {
                Value::Byte(v as u8)
            } else {
                Value::Int(v)
            }
        }
        ("Float", "Float") => {
            let (x, y) = (a.as_float(), b.as_float());
            match op {
                "Add" => Value::Float(x + y),
                "Subtract" => Value::Float(x - y),
                "Multiply" => Value::Float(x * y),
                // execDivide_FloatFloat: A / B without a test (FDIV).
                "Divide" => Value::Float(x / y),
                // execPercent_FloatFloat: appFmod in double.
                "Percent" => Value::Float((x as f64 % y as f64) as f32),
                // execMultiplyMultiply_FloatFloat: appPow in double.
                "MultiplyMultiply" => Value::Float((x as f64).dpowf(y as f64) as f32),
                "Less" => Value::Bool(x < y),
                "Greater" => Value::Bool(x > y),
                "LessEqual" => Value::Bool(x <= y),
                "GreaterEqual" => Value::Bool(x >= y),
                "EqualEqual" => Value::Bool(x == y),
                "NotEqual" => Value::Bool(x != y),
                // execComplementEqual_FloatFloat: |A − B| < 1e-4 (double).
                "ComplementEqual" => Value::Bool((((x - y).abs()) as f64) < 1e-4),
                _ => return None,
            }
        }
        ("Bool", "Bool") => {
            let (x, y) = (a.as_bool(), b.as_bool());
            Value::Bool(match op {
                "AndAnd" => x && y,
                "OrOr" => x || y,
                "XorXor" => x ^ y,
                "EqualEqual" => x == y,
                "NotEqual" => x != y,
                _ => return None,
            })
        }
        ("Str", "Str") => {
            let (x, y) = (a.as_name(), b.as_name());
            match op {
                "Concat" => Value::Str(x + &y),
                "At" => Value::Str(format!("{x} {y}")),
                "Less" => Value::Bool(x < y),
                "Greater" => Value::Bool(x > y),
                "LessEqual" => Value::Bool(x <= y),
                "GreaterEqual" => Value::Bool(x >= y),
                "EqualEqual" => Value::Bool(x == y),
                "NotEqual" => Value::Bool(x != y),
                "ComplementEqual" => Value::Bool(x.eq_ignore_ascii_case(&y)),
                _ => return None,
            }
        }
        ("Name", "Name") => {
            let eq = a.as_name().eq_ignore_ascii_case(&b.as_name());
            Value::Bool(match op {
                "EqualEqual" => eq,
                "NotEqual" => !eq,
                _ => return None,
            })
        }
        ("Object", "Object") => {
            let same = match (a, b) {
                (Value::Asset(x), Value::Asset(y)) => x.eq_ignore_ascii_case(y),
                (Value::Class(x), Value::Class(y)) => x == y,
                (Value::Object(x), Value::Object(y)) => x == y,
                (Value::Object(None), Value::Class(None)) | (Value::Class(None), Value::Object(None)) => true,
                _ => false,
            };
            Value::Bool(match op {
                "EqualEqual" => same,
                "NotEqual" => !same,
                _ => return None,
            })
        }
        ("Vector", "Vector") => {
            let (x, y) = (a.as_vector(), b.as_vector());
            match op {
                "Add" => vec([x[0] + y[0], x[1] + y[1], x[2] + y[2]]),
                "Subtract" => vec([x[0] - y[0], x[1] - y[1], x[2] - y[2]]),
                "Multiply" => vec([x[0] * y[0], x[1] * y[1], x[2] * y[2]]),
                "EqualEqual" => Value::Bool(x == y),
                "NotEqual" => Value::Bool(x != y),
                "Dot" => Value::Float(core_dot(x, y)),
                "Cross" => {
                    // Products and differences on the x87 stack (extended exponent), stored as float.
                    let d = |p: f32, q: f32, r: f32, t: f32| x87_24(x87_24(p as f64 * q as f64) - x87_24(r as f64 * t as f64)) as f32;
                    vec([d(x[1], y[2], x[2], y[1]), d(x[2], y[0], x[0], y[2]), d(x[0], y[1], x[1], y[0])])
                }
                _ => return None,
            }
        }
        ("Color", "Float") | ("Float", "Color") => {
            let (c, s) = if ta == "Color" { (a, b.as_float()) } else { (b, a.as_float()) };
            match (op, c) {
                // AActor::execMultiply_ColorFloat / execMultiply_FloatColor:
                // R, G then B through fild × F and _ftol (truncation on 64 bits: 0 out of range),
                // low byte kept (unclamped);
                // A stays the colour's. Fields in the order B, G, R, A.
                ("Multiply", Value::Struct(name, fields)) => Value::Struct(
                    name.clone(),
                    fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| if i < 3 { Value::Byte(crate::value::ftol((f.as_byte() as i32) as f32 * s) as u8) } else { f.clone() })
                        .collect(),
                ),
                _ => return None,
            }
        }
        ("Color", "Color") => match (op, a, b) {
            // AActor::execAdd_ColorColor / execSubtract_ColorColor:
            // R, G, B as bytes (8-bit add / sub, unclamped); A stays A's.
            ("Add" | "Subtract", Value::Struct(name, x), Value::Struct(_, y)) => Value::Struct(
                name.clone(),
                x.iter()
                    .zip(y)
                    .enumerate()
                    .map(|(i, (p, q))| {
                        if i >= 3 {
                            return p.clone();
                        }
                        let (p, q) = (p.as_byte(), q.as_byte());
                        Value::Byte(if op == "Add" { p.wrapping_add(q) } else { p.wrapping_sub(q) })
                    })
                    .collect(),
            ),
            _ => return None,
        },
        ("Vector", "Float") | ("Float", "Vector") => {
            let (v, s) = if ta == "Vector" { (a.as_vector(), b.as_float()) } else { (b.as_vector(), a.as_float()) };
            match op {
                "Multiply" => vec(v.map(|c| c * s)),
                // execDivide_VectorFloat: V × (1 / F), without a test.
                "Divide" => {
                    let k = 1.0 / s;
                    vec(v.map(|c| c * k))
                }
                _ => return None,
            }
        }
        ("Vector", "Rotator") => {
            let (v, m) = (a.as_vector(), rot_axes(b.as_rotator()));
            match op {
                // execGreaterGreater_VectorRotator: V.TransformVectorBy(
                // GMath.UnitCoords * R); execLessLess_VectorRotator
                // V.TransformVectorBy(GMath.UnitCoords / R).
                "GreaterGreater" => {
                    let c = openxiii_assets::coords::mul_rotator(&openxiii_assets::coords::UNIT, b.as_rotator());
                    vec(openxiii_assets::coords::transform_vector(v, &c))
                }
                "LessLess" => vec([dot(v, m[0]), dot(v, m[1]), dot(v, m[2])]),
                _ => return None,
            }
        }
        ("Rotator", "Rotator") => {
            let (x, y) = (a.as_rotator(), b.as_rotator());
            match op {
                "Add" => rot([x[0].wrapping_add(y[0]), x[1].wrapping_add(y[1]), x[2].wrapping_add(y[2])]),
                "Subtract" => rot([x[0].wrapping_sub(y[0]), x[1].wrapping_sub(y[1]), x[2].wrapping_sub(y[2])]),
                "EqualEqual" => Value::Bool(x == y),
                "NotEqual" => Value::Bool(x != y),
                _ => return None,
            }
        }
        ("Rotator", "Float") | ("Float", "Rotator") => {
            let (r, s) = if ta == "Rotator" { (a.as_rotator(), b.as_float()) } else { (b.as_rotator(), a.as_float()) };
            match op {
                // execMultiply_RotatorFloat: _ftol(FILD × F);
                // execDivide_RotatorFloat: _ftol(FILD × (1 / F)), without a test.
                "Multiply" => rot(r.map(|c| ftol(int_mul(c, s)))),
                "Divide" => {
                    let k = 1.0 / s;
                    rot(r.map(|c| ftol(int_mul(c, k))))
                }
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// First two pieces of "IntFloat" (["Int", "Float"]), without a list.
fn split_types(s: &str) -> [Option<&str>; 2] {
    let mut out = [None, None];
    let mut n = 0;
    let mut start = 0;
    for (i, c) in s.char_indices().skip(1) {
        if c.is_ascii_uppercase() {
            if n < 2 {
                out[n] = Some(&s[start..i]);
            }
            n += 1;
            start = i;
        }
    }
    if start < s.len() && n < 2 {
        out[n] = Some(&s[start..]);
    }
    out
}

/// Compound operators (`+=`…): return the new value and write it into args[0].
fn assign_op(op: &str, ty: &str, args: &mut [Value]) -> Option<Value> {
    let base = op.strip_suffix("Equal")?;
    // <=, >=, ==, != also end with "Equal" but are not assignments.
    if matches!(base, "Less" | "Greater" | "Equal" | "Not" | "ComplementEqual" | "Complement") {
        return None;
    }
    let [ta, tb] = split_types(ty);
    let (ta, tb) = (ta?, tb?);
    let result = match (ta, tb) {
        ("Int", "Float") | ("Byte", "Float") => {
            // execMultiplyEqual_IntFloat: _ftol(FILD × F);
            // execDivideEqual_IntFloat: _ftol(FILD / F), 0 if F is zero.
            let (i, g) = (args[0].as_int(), args[1].as_float());
            let f = match base {
                "Multiply" => int_mul(i, g),
                "Divide" => {
                    if g == 0.0 {
                        0.0
                    } else {
                        (i as f64 / g as f64) as f32
                    }
                }
                _ => return None,
            };
            if ta == "Byte" {
                Value::Byte(ftol(f) as u8)
            } else {
                Value::Int(ftol(f))
            }
        }
        // execDivideEqual_ByteByte: B zero → result 0, A unchanged.
        ("Byte", "Byte") if base == "Divide" && args[1].as_int() & 0xff == 0 => return Some(Value::Byte(0)),
        _ => binary_types(base, ta, tb, &args[0], &args[1])?,
    };
    args[0] = result.clone();
    Some(result)
}

/// Object operator resolved once per function (Vm::call_prep): same computation as `operator`
/// for operators without an out parameter, without reading the name again at each call
/// (the result is the same as `operator` for each name).
#[derive(Clone, Copy)]
pub(crate) enum OpFast {
    Un(fn(&Value) -> Value),
    Bin(fn(&Value, &Value) -> Value),
}

/// `name`: name of an Object function; `arity`: number of its parameters. None: the call
/// goes through `operator` as before (compound operators, ++, --, strings, names, colours…).
pub(crate) fn fast_operator(name: &str, arity: usize) -> Option<OpFast> {
    let (op, ty) = name.split_once('_')?;
    // Same order as `operator`: ++ and -- first, then the prefixes, then the assignments.
    if op == "AddAdd" || op == "SubtractSubtract" {
        return None;
    }
    if let Some(t) = ty.strip_prefix("Pre") {
        if arity != 1 {
            return None;
        }
        let f: fn(&Value) -> Value = match (op, t) {
            ("Not", _) => |a| Value::Bool(!a.as_bool()),
            ("Complement", _) => |a| Value::Int(!a.as_int()),
            ("Subtract", "Int") => |a| Value::Int(a.as_int().wrapping_neg()),
            ("Subtract", "Float") => |a| Value::Float(-a.as_float()),
            ("Subtract", "Vector") => |a| vec(a.as_vector().map(|c| -c)),
            _ => return None,
        };
        return Some(OpFast::Un(f));
    }
    if arity != 2 {
        return None;
    }
    // assign_op only returns None for a name ending in "Equal" for these five comparisons.
    if op.ends_with("Equal") && !matches!(op, "LessEqual" | "GreaterEqual" | "EqualEqual" | "NotEqual" | "ComplementEqual") {
        return None;
    }
    let [ta, tb] = split_types(ty);
    let f: fn(&Value, &Value) -> Value = match (ta.unwrap_or(""), tb.unwrap_or(""), op) {
        ("Int", "Int", "Add") => |a, b| Value::Int(a.as_int().wrapping_add(b.as_int())),
        ("Int", "Int", "Subtract") => |a, b| Value::Int(a.as_int().wrapping_sub(b.as_int())),
        ("Int", "Int", "Multiply") => |a, b| Value::Int(a.as_int().wrapping_mul(b.as_int())),
        ("Int", "Int", "Divide") => |a, b| {
            let (x, y) = (a.as_int(), b.as_int());
            Value::Int(if y == 0 { 0 } else { x.wrapping_div(y) })
        },
        ("Int", "Int", "LessLess") => |a, b| Value::Int(a.as_int().wrapping_shl(b.as_int() as u32)),
        ("Int", "Int", "GreaterGreater") => |a, b| Value::Int(a.as_int().wrapping_shr(b.as_int() as u32)),
        ("Int", "Int", "GreaterGreaterGreater") => |a, b| Value::Int(((a.as_int() as u32).wrapping_shr(b.as_int() as u32)) as i32),
        ("Int", "Int", "And") => |a, b| Value::Int(a.as_int() & b.as_int()),
        ("Int", "Int", "Or") => |a, b| Value::Int(a.as_int() | b.as_int()),
        ("Int", "Int", "Xor") => |a, b| Value::Int(a.as_int() ^ b.as_int()),
        ("Int", "Int", "Less") | ("Byte", "Byte", "Less") => |a, b| Value::Bool(a.as_int() < b.as_int()),
        ("Int", "Int", "Greater") | ("Byte", "Byte", "Greater") => |a, b| Value::Bool(a.as_int() > b.as_int()),
        ("Int", "Int", "LessEqual") | ("Byte", "Byte", "LessEqual") => |a, b| Value::Bool(a.as_int() <= b.as_int()),
        ("Int", "Int", "GreaterEqual") | ("Byte", "Byte", "GreaterEqual") => |a, b| Value::Bool(a.as_int() >= b.as_int()),
        ("Int", "Int", "EqualEqual") | ("Byte", "Byte", "EqualEqual") => |a, b| Value::Bool(a.as_int() == b.as_int()),
        ("Int", "Int", "NotEqual") | ("Byte", "Byte", "NotEqual") => |a, b| Value::Bool(a.as_int() != b.as_int()),
        ("Float", "Float", "Add") => |a, b| Value::Float(a.as_float() + b.as_float()),
        ("Float", "Float", "Subtract") => |a, b| Value::Float(a.as_float() - b.as_float()),
        ("Float", "Float", "Multiply") => |a, b| Value::Float(a.as_float() * b.as_float()),
        ("Float", "Float", "Divide") => |a, b| Value::Float(a.as_float() / b.as_float()),
        ("Float", "Float", "Less") => |a, b| Value::Bool(a.as_float() < b.as_float()),
        ("Float", "Float", "Greater") => |a, b| Value::Bool(a.as_float() > b.as_float()),
        ("Float", "Float", "LessEqual") => |a, b| Value::Bool(a.as_float() <= b.as_float()),
        ("Float", "Float", "GreaterEqual") => |a, b| Value::Bool(a.as_float() >= b.as_float()),
        ("Float", "Float", "EqualEqual") => |a, b| Value::Bool(a.as_float() == b.as_float()),
        ("Float", "Float", "NotEqual") => |a, b| Value::Bool(a.as_float() != b.as_float()),
        ("Bool", "Bool", "EqualEqual") => |a, b| Value::Bool(a.as_bool() == b.as_bool()),
        ("Bool", "Bool", "NotEqual") => |a, b| Value::Bool(a.as_bool() != b.as_bool()),
        ("Object", "Object", "EqualEqual") => |a, b| Value::Bool(objects_equal(a, b)),
        ("Object", "Object", "NotEqual") => |a, b| Value::Bool(!objects_equal(a, b)),
        ("Vector", "Vector", "Add") => |a, b| {
            let (x, y) = (a.as_vector(), b.as_vector());
            vec([x[0] + y[0], x[1] + y[1], x[2] + y[2]])
        },
        ("Vector", "Vector", "Subtract") => |a, b| {
            let (x, y) = (a.as_vector(), b.as_vector());
            vec([x[0] - y[0], x[1] - y[1], x[2] - y[2]])
        },
        ("Vector", "Vector", "EqualEqual") => |a, b| Value::Bool(a.as_vector() == b.as_vector()),
        ("Vector", "Vector", "NotEqual") => |a, b| Value::Bool(a.as_vector() != b.as_vector()),
        ("Vector", "Vector", "Dot") => |a, b| Value::Float(core_dot(a.as_vector(), b.as_vector())),
        ("Vector", "Float", "Multiply") => |a, b| {
            let s = b.as_float();
            vec(a.as_vector().map(|c| c * s))
        },
        ("Float", "Vector", "Multiply") => |a, b| {
            let s = a.as_float();
            vec(b.as_vector().map(|c| c * s))
        },
        ("Vector", "Float", "Divide") => |a, b| {
            let k = 1.0 / b.as_float();
            vec(a.as_vector().map(|c| c * k))
        },
        ("Rotator", "Rotator", "Add") => |a, b| {
            let (x, y) = (a.as_rotator(), b.as_rotator());
            rot([x[0].wrapping_add(y[0]), x[1].wrapping_add(y[1]), x[2].wrapping_add(y[2])])
        },
        ("Rotator", "Rotator", "Subtract") => |a, b| {
            let (x, y) = (a.as_rotator(), b.as_rotator());
            rot([x[0].wrapping_sub(y[0]), x[1].wrapping_sub(y[1]), x[2].wrapping_sub(y[2])])
        },
        ("Rotator", "Rotator", "EqualEqual") => |a, b| Value::Bool(a.as_rotator() == b.as_rotator()),
        ("Rotator", "Rotator", "NotEqual") => |a, b| Value::Bool(a.as_rotator() != b.as_rotator()),
        ("Rotator", "Float", "Multiply") => |a, b| {
            let s = b.as_float();
            rot(a.as_rotator().map(|c| ftol(int_mul(c, s))))
        },
        ("Float", "Rotator", "Multiply") => |a, b| {
            let s = a.as_float();
            rot(b.as_rotator().map(|c| ftol(int_mul(c, s))))
        },
        _ => return None,
    };
    Some(OpFast::Bin(f))
}

/// Equality of execEqualEqual_ObjectObject as `binary_types` computes it.
fn objects_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Asset(x), Value::Asset(y)) => x.eq_ignore_ascii_case(y),
        (Value::Class(x), Value::Class(y)) => x == y,
        (Value::Object(x), Value::Object(y)) => x == y,
        (Value::Object(None), Value::Class(None)) | (Value::Class(None), Value::Object(None)) => true,
        _ => false,
    }
}

#[path = "native_routes.rs"]
mod native_routes;
pub(crate) use native_routes::NativeRoute;

/// Native families. The discriminants are also development trace identifiers.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dispatcher {
    Canvas,
    Anim,
    Cutscene,
    Bot,
    Ai,
    Gui,
    Video,
    Ipdrv,
    Player,
    Uweb,
    StatLog,
    Solo,
}

impl Dispatcher {

    /// Tries this dispatcher: Some = it answered. The canvas dispatcher only answers for the
    /// canvas object.
    #[inline]
    fn try_call(self, vm: &mut Vm, name: &str, this: ObjId, args: &mut [Value], supplied: &[bool]) -> Result<Option<Value>> {
        Ok(match self {
            Dispatcher::Canvas => {
                if vm.canvas != Some(this) {
                    return Ok(None);
                }
                vm.canvas_native(name, this, args, supplied)
            }
            Dispatcher::Anim => vm.anim_native(name, this, args, supplied),
            Dispatcher::Cutscene => vm.cutscene_native(name, this, args, supplied),
            Dispatcher::Bot => vm.bot_native(name, this, args, supplied),
            Dispatcher::Ai => vm.ai_native(name, this, args, supplied),
            Dispatcher::Gui => vm.gui_native(name, this, args, supplied),
            Dispatcher::Video => vm.video_native(name, this, args),
            Dispatcher::Ipdrv => vm.ipdrv_native(name, this, args),
            Dispatcher::Player => vm.player_native(name, this, args),
            Dispatcher::Uweb => vm.uweb_native(name, this, args),
            Dispatcher::StatLog => vm.stat_log_native(name, this, args),
            Dispatcher::Solo => return vm.solo_native(name, this, args, supplied),
        })
    }
}

/// Resolve the name once per prepared script function. Class restrictions use its actual receiver at invocation.
pub(crate) fn dispatch(name: &str) -> NativeRoute {
    native_routes::lookup(name)
}

pub fn call(vm: &mut Vm, name: &str, this: ObjId, args: &mut [Value], supplied: &[bool]) -> Result<Value> {
    call_dispatches(vm, name, dispatch(name), this, args, supplied)
}

macro_rules! answered {
    ($name:expr, $who:expr) => {
    };
}

/// Invoke the registered family, then preserve the shared operator and core fallbacks.
pub(crate) fn call_dispatches(vm: &mut Vm, name: &str, route: NativeRoute, this: ObjId, args: &mut [Value], supplied: &[bool]) -> Result<Value> {
    crate::tally::inc(crate::tally::NATIVES);
    // ATrail::execInit, execReset, execAddSection.
    let trail = route == NativeRoute::Trail;
    if trail && vm.is_a(this, "Trail") {
        match name {
            "Init" => vm.trail_init(this),
            "Reset" => vm.trail_reset(this),
            _ => vm.trail_add_section_native(this, args[0].as_vector()),
        }
        answered!(name, crate::dispatch_trace::TRAIL);
        return Ok(Value::Void);
    }
    if let Some(handler) = route.handler(vm, this) {
        if let Some(v) = handler.try_call(vm, name, this, args, supplied)? {
            answered!(name, handler as u8);
            return Ok(v);
        }
    }
    if let Some(r) = operator(name, args) {
        answered!(name, crate::dispatch_trace::OPERATOR);
        return r;
    }
    answered!(name, crate::dispatch_trace::REST);
    rest_value(vm, name, this, args, supplied)
}

/// Object operators ("Op_Types"): none of the dispatchers before them in `call`
/// knows a name with "_", so a call from call_inner can come here directly for an
/// Object function (same result, same writes into `args`). None: not a known operator.
pub(crate) fn operator(name: &str, args: &mut [Value]) -> Option<Result<Value>> {
    if let Some((op, ty)) = name.split_once('_') {
        // Prefix and postfix ++ / --.
        if op == "AddAdd" || op == "SubtractSubtract" {
            let delta = if op == "AddAdd" { 1 } else { -1 };
            let old = args[0].clone();
            let new = match old {
                Value::Byte(b) => Value::Byte(b.wrapping_add(delta as u8)),
                ref v => Value::Int(v.as_int().wrapping_add(delta)),
            };
            args[0] = new.clone();
            return Some(Ok(if ty.starts_with("Pre") { new } else { old }));
        }
        if let Some(t) = ty.strip_prefix("Pre") {
            return Some(Ok(match (op, t) {
                ("Not", _) => Value::Bool(!args[0].as_bool()),
                ("Complement", _) => Value::Int(!args[0].as_int()),
                ("Subtract", "Int") => Value::Int(args[0].as_int().wrapping_neg()),
                ("Subtract", "Float") => Value::Float(-args[0].as_float()),
                ("Subtract", "Vector") => vec(args[0].as_vector().map(|c| -c)),
                _ => return Some(Err(VmError::Native(format!("{name} not implemented")))),
            }));
        }
        if args.len() >= 2 {
            if let Some(v) = assign_op(op, ty, args) {
                return Some(Ok(v));
            }
            if let Some(v) = binary(op, ty, &args[0], &args[1]) {
                return Some(Ok(v));
            }
        }
    }
    None
}

/// The LevelInfo lists store an unfiltered object link, including a recently freed actor.
#[derive(Clone, Copy)]
enum LevelActorList {
    Controllers,
    Pawns,
}

impl LevelActorList {
    fn head(self) -> &'static str {
        match self {
            Self::Controllers => "ControllerList",
            Self::Pawns => "PawnList",
        }
    }

    fn next(self, vm: &Vm, object: ObjId) -> &Value {
        match self {
            Self::Controllers => vm.native_prop(object, crate::NativeField::NextController),
            Self::Pawns => vm.propr(object, "nextPawn"),
        }
    }

    fn set_next(self, vm: &mut Vm, object: ObjId, value: Value) {
        match self {
            Self::Controllers => vm.set_native_prop(object, crate::NativeField::NextController, value),
            Self::Pawns => vm.set_prop(object, "nextPawn", value),
        }
    }
}

/// End of `call`: natives of Object and of the other classes, after the dispatchers and the operators.
fn rest_value(vm: &mut Vm, name: &str, this: ObjId, args: &mut [Value], supplied: &[bool]) -> Result<Value> {
    let f = |i: usize| args.get(i).map(|v| v.as_float()).unwrap_or(0.0);
    let n = |i: usize| args.get(i).map(|v| v.as_int()).unwrap_or(0);
    let s = |i: usize| args.get(i).map(|v| v.as_name()).unwrap_or_default();
    Ok(match name {
        // execLog: GLog->Logf(Tag, "%s", S), Tag being NAME_ScriptLog (0x302)
        // if it is omitted; a None Tag passed by the script stays None.
        "Log" | "DebugLog" => {
            let msg = text(vm, &args[0]);
            let tag = if supplied.get(1) == Some(&true) {
                Some(s(1)).filter(|t| !t.is_empty()).unwrap_or_else(|| "None".into())
            } else {
                "ScriptLog".into()
            };
            vm.log.push(format!("{tag}: {msg}"));
            Value::Void
        }
        // execWarn: Stack.Logf("%s", S), script warning (NAME_ScriptWarning
        // with the object and the function) like an Accessed None.
        "Warn" => {
            let msg = text(vm, &args[0]);
            vm.native_warn(&msg);
            Value::Void
        }
        // UXboxLiveManager::execConvertString / execUnconvertString: with
        // the UXboxLive subsystem (XboxLiveDevice=Engine.XboxLive, Init on PC too), the
        // spaces become "_" or the reverse.
        "ConvertString" => Value::Str(s(0).replace(' ', "_")),
        "UnconvertString" => Value::Str(s(0).replace('_', " ")),
        // AInternetLink::execIpAddrToString: "%i.%i.%i.%i:%i" of Addr (high byte
        // first) and Port.
        "IpAddrToString" => {
            let (addr, port) = match args.first() {
                Some(Value::Struct(_, f)) => (f.first().map_or(0, |v| v.as_int()) as u32, f.get(1).map_or(0, |v| v.as_int())),
                _ => (0, 0),
            };
            Value::Str(format!("{}.{}.{}.{}:{port}", addr >> 24, (addr >> 16) & 0xff, (addr >> 8) & 0xff, addr & 0xff))
        }
        // UObject::execRand: appRand() % Max.
        "Rand" => Value::Int(if n(0) > 0 { vm.app_rand_script() % n(0) } else { 0 }),
        "FRand" => Value::Float(vm.rand()),
        "RandRange" => Value::Float(f(0) + (f(1) - f(0)) * vm.rand()),
        "Min" => Value::Int(n(0).min(n(1))),
        "Max" => Value::Int(n(0).max(n(1))),
        // execClamp / execFClamp: A if V < A, otherwise V if V < B, otherwise B.
        "Clamp" => Value::Int(if n(0) < n(1) {
            n(1)
        } else if n(0) < n(2) {
            n(0)
        } else {
            n(2)
        }),
        // execFMin: A if A <= B, otherwise B (B if either is NaN).
        "FMin" => Value::Float(if f(0) <= f(1) { f(0) } else { f(1) }),
        // execFMax: B if A < B or unordered, otherwise A.
        // Not in the original: an enhanced LAN client lowers ReplicateMove's floor
        // (Vm::replicate_move_floor; B unchanged otherwise).
        "FMax" => {
            let (a, b) = (f(0), vm.replicate_move_floor(f(1)));
            Value::Float(if a >= b { a } else { b })
        }
        // execFClamp: Min if V < Min, otherwise V if V < Max, otherwise Max.
        "FClamp" => Value::Float(if f(0) < f(1) {
            f(1)
        } else if f(0) < f(2) {
            f(0)
        } else {
            f(2)
        }),
        // execLerp: A + (B - A) * Alpha.
        "Lerp" => Value::Float(x87_24(x87_24(x87_24(f(2) as f64 - f(1) as f64) * f(0) as f64) + f(1) as f64) as f32),
        // execSmerp: a² = Alpha², s = 3a² − (a² Alpha + a² Alpha), A + (B − A) s.
        // Everything stays on the x87 stack (extended exponent) until the final fstp: no
        // intermediate float overflow (Alpha = 1e30 included).
        "Smerp" => {
            let (al, a, b) = (f(0) as f64, f(1) as f64, f(2) as f64);
            let a2 = x87_24(al * al);
            let a3 = x87_24(a2 * al);
            let s = x87_24(x87_24(3.0 * a2) - x87_24(a3 + a3));
            Value::Float(x87_24(x87_24(x87_24(b - a) * s) + a) as f32)
        }
        // execAbs: fchs if A < 0 (or NaN); −0 stays −0.
        "Abs" => Value::Float(if f(0) >= 0.0 { f(0) } else { -f(0) }),
        // execSin… (and following): Core's appSin / appCos / appTan / appAtan / appLoge
        // (fsin, fcos, fptan, fpatan, fyl2x on the double), result rounded to float.
        // fsin / fcos leave the operand unchanged (C2) when |x| >= 2^63.
        "Sin" => Value::Float(if f(0).abs() >= 9.223372e18 && f(0).is_finite() { f(0) } else { (f(0) as f64).dsin() as f32 }),
        "Cos" => Value::Float(if f(0).abs() >= 9.223372e18 && f(0).is_finite() { f(0) } else { (f(0) as f64).dcos() as f32 }),
        "Tan" => Value::Float((f(0) as f64).dtan() as f32),
        // execAtan: fpatan(x, 1).
        "Atan" => Value::Float((f(0) as f64).datan() as f32),
        // execAcos: appAcos, msvcr70's _CIacos.
        "Acos" => Value::Float(x87_acos(f(0))),
        // execExp: appExp.
        "Exp" => Value::Float(x87_exp(f(0))),
        "Loge" => Value::Float((f(0) as f64).dln() as f32),
        // execSqrt: fsqrt, rounded to 24 bits (FPU in single precision).
        "Sqrt" => Value::Float(f(0).sqrt()),
        // execSquare: A * A.
        "Square" => Value::Float(f(0) * f(0)),
        // execVSize: appSqrt((Z² + Y²) + X²).
        "VSize" => Value::Float({
            let v = args[0].as_vector();
            // The sum goes to appSqrt as a double: no infinity for |V| > 1.8e19.
            core_dot_x87(v, v).sqrt() as f32
        }),
        // execNormal: FVector::SafeNormal.
        "Normal" => vec(safe_normal(args[0].as_vector())),
        // execVRand: point drawn in the cube until it falls in the unit ball,
        // then normalized without a test (UnsafeNormal).
        "VRand" => {
            let v = loop {
                let v = [vm.rand() * 2.0 - 1.0, vm.rand() * 2.0 - 1.0, vm.rand() * 2.0 - 1.0];
                // Loop on (Z² + Y²) + X² > 1.
                if (v[2] * v[2] + v[1] * v[1]) + v[0] * v[0] <= 1.0 {
                    break v;
                }
            };
            let k = app_rsqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
            vec(v.map(|c| k * c))
        }
        // execMirrorVectorByNormal: V − 2N (V·N) with N normalized (SafeNormal);
        // V·N summed as (Y·Ny + Z·Nz) + X·Nx.
        "MirrorVectorByNormal" => {
            let (v, nrm) = (args[0].as_vector(), safe_normal(args[1].as_vector()));
            let d = (v[1] * nrm[1] + v[2] * nrm[2]) + v[0] * nrm[0];
            vec([v[0] - (nrm[0] + nrm[0]) * d, v[1] - (nrm[1] + nrm[1]) * d, v[2] - (nrm[2] + nrm[2]) * d])
        }
        // execGetAxes: axes of GMath.UnitCoords / R;
        // execGetUnAxes: GMath.UnitCoords * R, not the transpose.
        "GetAxes" | "GetUnAxes" => {
            let r = args[0].as_rotator();
            let axes = if name == "GetAxes" {
                rot_axes(r)
            } else {
                let c = openxiii_assets::coords::mul_rotator(&openxiii_assets::coords::UNIT, r);
                [c.x, c.y, c.z]
            };
            for k in 0..3 {
                vec_in(&mut args[k + 1], axes[k]);
            }
            Value::Void
        }
        "Normalize" => rot(args[0].as_rotator().map(|c| ((c & 0xFFFF) as i16) as i32)),
        // execRotRand: (appRand() * 2) % 0xFFFF, yaw drawn before pitch.
        "RotRand" => {
            let yaw = (vm.app_rand() << 1) % 0xffff;
            let pitch = (vm.app_rand() << 1) % 0xffff;
            let roll = if args.first().is_some_and(|b| b.as_bool()) { (vm.app_rand() << 1) % 0xffff } else { 0 };
            rot([pitch, yaw, roll])
        }
        // execLen: ArrayNum − 1 of the FString (0 if empty).
        "Len" => Value::Int(s(0).chars().count() as i32),
        "InStr" => Value::Int(s(0).find(&s(1)).map_or(-1, |i| s(0)[..i].chars().count() as i32)),
        // execMid: Count is 0xFFFF by default; FString::Mid
        // compares as unsigned (a negative start gives an empty string).
        "Mid" => {
            let chars: Vec<char> = s(0).chars().collect();
            let len = chars.len() as u32;
            let count = if supplied.get(2) == Some(&true) { n(2) } else { 0xffff };
            let end = count.wrapping_add(n(1)) as u32;
            let start = if (n(1) as u32) < len { n(1) as u32 } else { len };
            let stop = if start <= end { end.min(len) } else { start };
            Value::Str(chars[start as usize..stop as usize].iter().collect())
        }
        // execLeft: FString::Left, Count clamped to [0, Len];
        // execRight: FString::Right, same.
        "Left" => Value::Str(s(0).chars().take(n(1).max(0) as usize).collect()),
        "Right" => {
            let chars: Vec<char> = s(0).chars().collect();
            let k = (n(1).max(0) as usize).min(chars.len());
            Value::Str(chars[chars.len() - k..].iter().collect())
        }
        // FString::Caps / Locs: letters a to z only.
        "Caps" => Value::Str(s(0).chars().map(|c| c.to_ascii_uppercase()).collect()),
        "Lowcaps" => Value::Str(s(0).chars().map(|c| c.to_ascii_lowercase()).collect()),
        // execChr: the low byte; execAsc: the first
        // character as a signed char.
        "Chr" => Value::Str(match n(0) & 0xff {
            0 => String::new(),
            b => char::from_u32(b as u32).map(String::from).unwrap_or_default(),
        }),
        "Asc" => Value::Int(s(0).chars().next().map_or(0, |c| if (c as u32) < 256 { c as u32 as u8 as i8 as i32 } else { c as i32 })),
        // UObject::execLocalize: Localize(Section, Key, Package).
        "Localize" => Value::Str(vm.localize(&s(0), &s(1), &s(2), false)),
        // UObject::execDynamicLoadObject → StaticLoadObject(ObjectClass, NULL,
        // Name) and execFindObject: the object with this path and this class. A script
        // class is only returned if the requested class is Class: "MeshArmesPickup.Magnum"
        // requested as StaticMesh is the mesh, not the Magnum weapon class (otherwise the cast
        // StaticMesh(...) of InventoryAttachment.PostBeginPlay returns None and the weapon is invisible).
        "DynamicLoadObject" | "FindObject" => {
            let path = s(0);
            // The requested class: a VM class object, otherwise the last `class'Name'` constant evaluated (StaticMesh, Mesh, Font,
            // Class… are not VM classes: their constant arrives as Class(None)).
            let requested = match args.get(1) {
                Some(Value::Class(Some(c))) => vm.classes[*c].name.clone(),
                Some(Value::Class(None)) => vm.meta_const.take().unwrap_or_else(|| "Class".to_string()),
                _ => "Class".to_string(),
            };
            // A UClass is a Class, a State, a Struct, a Field, an Object: is the requested class one of them?
            let wants_class = dyn_class_is_a(vm, "Class", &requested);
            // UObject::execFindObject: StaticFindObject(Class, NULL, Name, false); without "." there is
            // no package (Outer NULL) and nothing is found. No game script calls it.
            if name == "FindObject" {
                if !path.contains('.') {
                    return Ok(Value::Object(None));
                }
                if let Some(v) = find_object_vm(vm, &path, args.get(1)) {
                    return Ok(v);
                }
            }
            let may_fail = supplied.get(2).copied().unwrap_or(false) && args.get(2).is_some_and(|a| a.as_bool());
            // UObject::ResolveName: "ini:Section.Key" takes the text of this key in XIII.ini; without it, ConfigNotFound
            // is raised, and the catch writes FailedLoadObject with an Outer "None" and an empty name.
            let mut path = path;
            if name == "DynamicLoadObject" && path.len() >= 4 && path.is_char_boundary(4) && path[..4].eq_ignore_ascii_case("ini:") {
                let value_value = path[4..].rsplit_once('.').and_then(|(section, key_tuple)| vm.config.as_mut()?.get_string(section, key_tuple, "System"));
                match value_value {
                    Some(v) => path = v,
                    None => {
                        if !may_fail {
                            let not_found = vm.localize("Errors", "ConfigNotFound", "Core", false);
                            let failed_object = vm.localize("Errors", "FailedLoadObject", "Core", false);
                            let err = crate::value::printf_s(&not_found, &[&path]);
                            let line = crate::value::printf_s(&failed_object, &[&requested, "None", "", &err]);
                            vm.log.push(format!("Warning: {line}"));
                        }
                        return Ok(if wants_class { Value::Class(None) } else { Value::Object(None) });
                    }
                }
            }
            // UObject::ResolveName: without ".", no package (InOuter null);
            // StaticLoadObject → GetPackageLinker(NULL, NULL) raises
            // PackageResolveFailed, written by SafeLoadError as "FailedLoad 'NULL'",
            // then StaticLoadObject's catch writes FailedLoadObject and returns None.
            // Both lines as NAME_Warning, except with MayFail (LOAD_Quiet 0x2000).
            if name == "DynamicLoadObject" && !path.contains('.') && !path.to_ascii_lowercase().starts_with("ini:") {
                if !may_fail {
                    // LoadFlags of DynamicLoadObject: LOAD_NoWarn, plus LOAD_Quiet with MayFail (lines missing).
                    let lines_list = failure_static_load(vm, &requested, &path, false);
                    vm.log.extend(lines_list);
                }
                return Ok(if wants_class { Value::Class(None) } else { Value::Object(None) });
            }
            // StaticLoadObject: emulated linker loading, PostLoads in its order (Reset of the
            // templates of the loaded classes, on the game's sequence).
            if name == "DynamicLoadObject" && vm.load_order.is_some() && !path.is_empty() {
                let class_name = Some(requested.clone());
                let events = vm.lo_run(|vm, lo| {
                    vm.lo_load(lo, &path, class_name.as_deref());
                });
                vm.apply_post_loads(&events, None);
            }
            // Class'Core.Class' (like Class'Engine.StaticMesh') is not a VM class and
            // arrives as Class(None): the script class is only taken if its package is that
            // of the path ("XIII.Magnum" is the class, "MeshArmesPickup.Magnum" a mesh).
            let package_of = |c: crate::value::ClassId| {
                // Mods build only: "Mod" for a mod's new class (mod_classes.rs).
                #[cfg(feature = "mods")]
                if let Some(p) = vm.mod_class_package(c) {
                    return p.to_string();
                }
                vm.packages[vm.classes[c].package].name.clone()
            };
            let found = vm.class(&path).filter(|&c| match path.split_once('.') {
                Some((pkg, _)) => package_of(c).eq_ignore_ascii_case(pkg),
                None => true,
            });
            match found {
                Some(c) if wants_class => Value::Class(Some(c)),
                _ if path.is_empty() => Value::Object(None),
                _ => match vm.script_object(&path) {
                    Some(id) => Value::Object(Some(id)),
                    None => {
                        // StaticLoadObject: the package's linker (GetPackageLinker) then the export with this name whose
                        // class is the requested class or derives from it (ULinkerLoad::Create → FindExportIndex, the group
                        // is not looked at). Missing package: PackageNotFound raised by GetPackageLinker; missing object: ObjectNotFound
                        // raised by StaticLoadObject. The catch writes FailedLoadObject (NAME_Warning unless MayFail) and returns None.
                        let short = path.rsplit_once('.').map_or(path.clone(), |(_, n)| n.to_string());
                        let package = path.split('.').next().unwrap_or("").to_string();
                        let (has_package, found_obj) = if vm.library.is_some() { dyn_find_export(vm, &package, &short, &requested) } else { (true, true) };
                        // Mods build only: a mod's own 3D model or texture ("Mod.<Name>", in no
                        // package) is found like the game's (a weapon attachment's StaticMeshName).
                        #[cfg(feature = "mods")]
                        let (has_package, found_obj) = if openxiii_assets::mod_meshes::get(&path).is_some() || openxiii_assets::mod_skeletal::get(&path).is_some() || openxiii_assets::mod_textures::get(&path).is_some() {
                            (true, true)
                        } else {
                            (has_package, found_obj)
                        };
                        if name == "DynamicLoadObject" && !(has_package && found_obj) {
                            if !may_fail {
                                let lines_list = failure_static_load(vm, &requested, &path, has_package);
                                vm.log.extend(lines_list);
                            }
                            if wants_class {
                                Value::Class(None)
                            } else {
                                Value::Object(None)
                            }
                        } else {
                            Value::Asset(path)
                        }
                    }
                },
            }
        }
        // execIsA: walks up Class then SuperField comparing the FName of
        // each class with the requested name (no class lookup).
        "IsA" => {
            let target = s(0);
            let mut class = vm.objects[this].as_ref().map(|o| o.class);
            while let Some(c) = class {
                if vm.classes[c].name.eq_ignore_ascii_case(&target) {
                    break;
                }
                class = vm.classes[c].super_class;
            }
            Value::Bool(class.is_some())
        }
        // execClassIsChildOf: false if either class is None.
        "ClassIsChildOf" => Value::Bool(match (args[0].as_class(), args[1].as_class()) {
            (Some(a), Some(b)) => vm.is_child_of(a, b),
            _ => false,
        }),
        "GotoState" => {
            // execGotoState: omitted state = current state, omitted label = None (Begin).
            let state = if supplied.first() == Some(&true) { s(0) } else { vm.objects[this].as_ref().and_then(|o| o.state.clone()).unwrap_or_default() };
            let label = if supplied.get(1) == Some(&true) { s(1) } else { String::new() };
            vm.goto_state(this, &state, &label)?;
            Value::Void
        }
        // ---- Engine: world, time, actors ----
        "PickStartPoint" => Value::Object(vm.pick_start_point(this)),
        // APawn::execPressingFire: Controller.bFire non-zero.
        "PressingFire" | "PressingAltFire" => {
            let controller = vm.propr(this, "Controller").as_object();
            let button = if name == "PressingFire" { "bFire" } else { "bAltFire" };
            Value::Bool(controller.is_some_and(|c| vm.propr(c, button).as_int() != 0))
        }
        "IncAttente" | "DecAttente" | "IncAlerte" | "DecAlerte" | "IncAttaque" | "DecAttaque" => {
            // ALevelInfo::execIncAttente: LevelInfo.MusicVars[0, 1, 2] (enemies
            // waiting, alert, attacking), which drive the music.
            use crate::canvas::AudioCall;
            let index = match &name[3..] {
                "Attente" => 0,
                "Alerte" => 1,
                _ => 2,
            };
            let delta = if name.starts_with("Inc") { 1 } else { -1 };
            // MusicVars(index).Value changes right away, with or without Audio; the subsystem
            // is only notified if it exists (Engine+0x50).
            let vars = vm.music_vars();
            if let Some((var, value, ty)) = vars.get(index).cloned() {
                let value = value.wrapping_add(delta);
                vm.set_music_var_value(index, value);
                let alert = vars.get(1).map_or(0, |v| v.1);
                vm.audio_dispatch(AudioCall::MusicCounter { index, delta, name: var, ty, value, alert });
            }
            Value::Void
        }
        "PauseMusic" | "PauseAllSounds" | "ResumeMusic" | "ResumeAllSounds" => {
            // AActor::execPauseMusic, execPauseAllSounds, execResumeMusic,
            // execResumeAllSounds: Audio->PauseSound / ResumeSound
            // (vtable +0xA8 / +0xAC) with (1, 0) for the music (sic: the effects category,
            // as in Engine.dll), (0xFFF, 0) for everything.
            use crate::canvas::AudioCall;
            let kinds = if name.ends_with("Music") { 1 } else { 0xfff };
            vm.audio_dispatch(if name.starts_with("Pause") { AudioCall::Pause { kinds } } else { AudioCall::Resume { kinds } });
            Value::Void
        }
        // AActor::execKillAllSounds: UHXAUDIOSubsystem::CleanUp,
        // PauseSound(0xFFF, 1), microphones, voices and the map's bank freed, without notifying the script.
        "KillAllSounds" => {
            vm.audio_clean_up();
            Value::Void
        }
        // AActor::execSetVolume: volume of all categories (0xFFF).
        "SetVolume" => {
            use crate::canvas::AudioCall;
            vm.audio_dispatch(AudioCall::Volume { kinds: 0xfff, volume: f(0) });
            Value::Void
        }
        // UForceFeedbackController::execEnableForceFeedback: passed to
        // UForceFeedbackManager::Instance (vtable +0x78) if it exists (absent from the original PC version, see
        // force_feedback.rs), with the player's index Player+0xA8.
        "EnableForceFeedback" => {
            let b = args.first().map_or(0, |v| i32::from(v.as_bool()));
            let client = vm.coop_feedback_client(this);
            if let Some(m) = vm.force_feedback.as_mut() {
                m.enable(client, b);
            }
            Value::Void
        }
        // AActor::execSetMusicSliderPos: Audio vtable +0xBC (position, 0 by default).
        "SetMusicSliderPos" => {
            use crate::canvas::AudioCall;
            vm.audio_dispatch(AudioCall::MusicPosition { pos: opt_int(&args, supplied, 0, 0) });
            Value::Void
        }
        // AActor::execRefreshLighting: XLevel->RefreshDynLight(this) (vt 0xF4).
        "RefreshLighting" => {
            vm.refresh_dyn_light(this);
            Value::Void
        }
        "Reset" => Value::Void,
        "PlayFiringSound" => {
            // AWeapon::execPlayFiringSound (Engine.dll): hFireSound (hNoAmmoSound if
            // bEmptyShot) played by the Instigator, attenuated by the weapon; parameters: silencer,
            // player + bIsSlave, 0.
            use crate::canvas::AudioCall;
            let silencer = opt_int(&args, supplied, 0, 0);
            let empty = vm.propr(this, "bEmptyShot").as_int() != 0;
            let sound = vm.prop(this, if empty { "hNoAmmoSound" } else { "hFireSound" });
            if let Value::Asset(sound) = sound {
                let instigator = vm.propr(this, "Instigator").as_object();
                let controller = instigator.and_then(|i| vm.propr(i, "Controller").as_object());
                let pc = vm.class("PlayerController");
                let player = controller.is_some_and(|c| match (vm.objects[c].as_ref().map(|o| o.class), pc) {
                    (Some(k), Some(t)) => vm.is_child_of(k, t),
                    _ => false,
                }) as i32;
                let slave = (vm.propr(this, "bIsSlave").as_int() != 0) as i32;
                let first = if empty { 0 } else { silencer };
                vm.audio_dispatch(AudioCall::Play { actor: instigator, rolloff: Some(this), sound, slot: 1, params: vec![first, player + slave, 0, -1, -1], check_handler: true, first_free: false });
            }
            Value::Void
        }
        // Sounds: arguments passed as is to the audio subsystem (openxiii-hx). AActor::execPlaySound,
        // execPlayVoice, execPlayRolloffSound: nothing without
        // a sound; Audio vtable +0xA0 (actor, attenuation, sound, category 1 / 2 / 1) if Param1 is -1
        // or if the sound has no handler (byte +0x29), otherwise +0x9C with Param1…5 and -1.
        "PlaySound" | "PlayOwnedSound" | "PlayVoice" | "PlayMenu" | "PlayMusic" | "PlayStrAmb" | "PlayRolloffSound" => {
            use crate::canvas::AudioCall;
            if let Value::Asset(sound) = &args[0] {
                let Some(sound) = crate::improved::navigation::menu_sound(vm, name, sound.clone()) else {
                    return Ok(Value::Void);
                };
                let (actor, rolloff, slot, first) = match name {
                    "PlayRolloffSound" => (Some(this), args.get(1).and_then(|a| a.as_object()), 1, 2),
                    "PlayVoice" => (Some(this), Some(this), 2, 1),
                    // AActor::execPlayStrAmb: +0xA0 (actor, actor, sound, 8).
                    "PlayStrAmb" => (Some(this), Some(this), 8, 9),
                    // AActor::execPlayMusic: without parameters, the actor itself.
                    "PlayMusic" if opt_int(&args, supplied, 1, -1) == -1 => (Some(this), Some(this), 0x10, 1),
                    "PlayMusic" => (None, None, 0x10, 1),
                    // AActor::execPlayMenu: Param1 at -1: +0xA0 (actor, actor, sound,
                    // category 0x20 by default); otherwise +0x9C (0, 0, …) without looking at the
                    // sound's handler.
                    "PlayMenu" if opt_int(&args, supplied, 2, -1) == -1 => (Some(this), Some(this), opt_int(&args, supplied, 1, 0x20), 2),
                    "PlayMenu" => (None, None, opt_int(&args, supplied, 1, 0x20), 2),
                    _ => (Some(this), Some(this), 1, 1),
                };
                let params: Vec<i32> = (first..first + 5).map(|i| opt_int(&args, supplied, i, -1)).collect();
                if name == "PlayMusic" {
                    vm.audio_saver_play_music(&sound, [params[0], params[1], params[2], params[3], params[4]]);
                }
                vm.audio_dispatch(AudioCall::Play { actor, rolloff, sound, slot, params, check_handler: name != "PlayMenu" && name != "PlayMusic", first_free: true });
            }
            Value::Void
        }
        // AActor::execPlaySndXIIIStep / execPlaySndPNJStep: nothing without
        // a step; Audio vtable +0xD4 / +0xD0 (actor, step, speed, 3rd and 4th parameters, 0 by default).
        "PlaySndXIIIStep" | "PlaySndPNJStep" => {
            use crate::canvas::AudioCall;
            if let Value::Asset(step) = &args[0] {
                let speed = args.get(1).map_or(0.0, |a| a.as_float());
                let third = opt_int(&args, supplied, 2, 0);
                let silent = args.get(3).is_some_and(|a| a.as_int() != 0);
                vm.audio_dispatch(if name == "PlaySndXIIIStep" {
                    AudioCall::XiiiStep { actor: this, step: step.clone(), speed, end_step: third, silent }
                } else {
                    AudioCall::NPCStep { actor: this, step: step.clone(), speed, category: third, silent }
                });
            }
            Value::Void
        }
        // AActor::execPlaySndPNJOno / execPlaySndDeathOno: nothing without
        // an onomatopoeia; Audio vtable +0xD8 / +0xDC (actor, ono, CodeMesh = 0 by default, timbre).
        "PlaySndPNJOno" | "PlaySndDeathOno" => {
            use crate::canvas::AudioCall;
            if let Value::Asset(ono) = &args[0] {
                vm.audio_dispatch(AudioCall::Ono {
                    actor: this,
                    ono: ono.clone(),
                    code_mesh: opt_int(&args, supplied, 1, 0),
                    timbre: opt_int(&args, supplied, 2, 0),
                    death: name == "PlaySndDeathOno",
                });
            }
            Value::Void
        }
        // AActor::execPlayStrVoice: Audio vtable +0x8C (actor, attenuation or
        // the actor itself, name); 0 without an audio subsystem.
        "PlayStrVoice" => {
            use crate::canvas::AudioCall;
            let text = args.first().map(|a| a.as_name()).unwrap_or_default();
            let rolloff = args.get(1).and_then(|a| a.as_object());
            Value::Bool(vm.audio_dispatch(AudioCall::StrVoice { actor: this, name: text, rolloff }))
        }
        // AActor::execStopSound, execStopMusic, execStopVoice,
        // execStopActorSounds, execStopAllSounds: Audio
        // vtable +0xA4 (actor, sound, categories, 1); StopSound does nothing without a sound.
        "StopSound" | "StopMusic" | "StopVoice" | "StopActorSounds" | "StopAllSounds" => {
            use crate::canvas::AudioCall;
            let call = match name {
                "StopSound" => match &args[0] {
                    Value::Asset(p) => Some(AudioCall::Stop { actor: Some(this), sound: Some(p.clone()), kinds: 1, notify: true }),
                    _ => None,
                },
                "StopMusic" => Some(AudioCall::Stop { actor: None, sound: None, kinds: 0x10, notify: true }),
                "StopVoice" => Some(AudioCall::Stop { actor: None, sound: None, kinds: 4, notify: true }),
                "StopActorSounds" => Some(AudioCall::Stop { actor: Some(this), sound: None, kinds: 1, notify: true }),
                _ => Some(AudioCall::Stop { actor: None, sound: None, kinds: 0xfff, notify: true }),
            };
            if let Some(c) = call {
                vm.audio_dispatch(c);
            }
            Value::Void
        }
        "DrawPlayerInfo" => {
            if let Some(canvas) = args[0].as_object() {
                vm.hud_draw_player_info(this, canvas);
            }
            Value::Void
        }
        "DrawAmmo" => {
            if let Some(canvas) = args[0].as_object() {
                let text = args[1].as_name();
                vm.hud_draw_ammo(this, canvas, &text);
            }
            Value::Void
        }
        "DrawWeaponsList" => {
            if let Some(canvas) = args[0].as_object() {
                vm.hud_draw_weapons_list(this, canvas);
            }
            Value::Void
        }
        "DrawStdBackground" => {
            if let Some(canvas) = args[0].as_object() {
                vm.hud_draw_std_background(this, canvas, args[1].as_float(), args[2].as_float());
            }
            Value::Void
        }
        "Initialize" => {
            // UInteraction::execInitialize: InitExecution then the Initialized event.
            // (UObject::InitExecution: the state frame already exists in the VM.)
            if let Some(f) = vm.find_function(this, "Initialized", false) {
                vm.call(f, this, vec![])?;
            }
            Value::Void
        }
        // URenderTargetMaterial::execUpdate: X, Y, width, height, CamLocation,
        // CamRotation, ViewFOV, FilterColor (0x00FFFFFF: white, A = 0 if it is omitted), HighLight and
        // FilterTexture, passed to URenderTargetMaterial::Update (vtable +0x94), which
        // renders the view right away into the texture (comic windows). The VM keeps the last view
        // of each rectangle and a call number: the display layer renders it when the number changes.
        "Update" if vm.is_a(this, "RenderTargetMaterial") => {
            let given = |i: usize| supplied.get(i) == Some(&true);
            // PlayerController+0x5B1 (bHighLight): HighLight > 0; +0x5B8: |HighLight|.
            let high_light = if given(8) { args[8].as_float() } else { 0.0 };
            vm.render_serial += 1;
            // UModel::PointRegion(CamLocation) then ULevel::GetZoneActor: zone with bForceClear →
            // WhiteTexture DrawTile of DistanceFogColor over the whole frame before the scene.
            let location = args[4].as_vector();
            let force_clear = vm
                .point_region(location)
                .0
                .filter(|&z| vm.propr(z, "bForceClear").as_bool())
                .map(|z| crate::canvas::color_of(&vm.prop(z, "DistanceFogColor")));
            let view = crate::canvas::RenderView {
                rect: [args[0].as_int(), args[1].as_int(), args[2].as_int(), args[3].as_int()],
                location,
                rotation: args[5].as_rotator(),
                fov: args[6].as_float(),
                filter: if given(7) { crate::canvas::color_of(&args[7]) } else { [255, 255, 255, 0] },
                highlight: high_light.abs(),
                high_light: high_light > 0.0,
                filter_texture: if given(9) { crate::canvas::material_path(&args[9]) } else { String::new() },
                serial: vm.render_serial,
                force_clear,
            };
            let size = (view.rect[2].max(1) as u32, view.rect[3].max(1) as u32);
            let (rotation, fov) = (view.rotation, view.fov);
            // Opaque filter (alpha 0xFF): the scene is not rendered (only
            // draws the filter), so no actor gets LastRenderTime there.
            let scene = view.filter[3] < 0xff;
            let views = vm.render_views.entry(this).or_default();
            views.retain(|v| v.rect != view.rect);
            views.push(view);
            if scene {
                vm.render_target_scene(this, location, rotation, fov, size);
            }
            Value::Void
        }
        // UParticleEmitter::execSpawnParticle and execSetMaxParticles.
        "SpawnParticle" if vm.is_a(this, "ParticleEmitter") => {
            let n = args.first().map_or(0, |v| v.as_int());
            vm.emitter_spawn_particle(this, n);
            Value::Void
        }
        // UParticleEmitter::execSetMaxParticles.
        "SetMaxParticles" if vm.is_a(this, "ParticleEmitter") => {
            let n = args.first().map_or(0, |v| v.as_int());
            vm.emitter_set_max_particles(this, n);
            Value::Void
        }
        "AllocRect" => {
            // URenderTargetMaterial::execAllocRect: free spot in the 32 × 32 grid
            // of 8-pixel cells (TexRes: one row of bits per row), searched column by
            // column then row by row; (−1, −1) without room, (0, 0) for a zero height.
            let (width, height) = (n(0), n(1));
            let mut rows: Vec<u32> = match vm.prop(this, "TexRes") {
                Value::Array(v) => v.iter().map(|x| x.as_int() as u32).collect(),
                _ => vec![0; 32],
            };
            rows.resize(32, 0);
            let (mut x, mut y) = (0, 0);
            if height >= 1 {
                let w = (width + 7) / 8;
                let h = (height + 7) / 8;
                let mut mask: u32 = 0;
                for i in 0..w {
                    mask |= 1u32.wrapping_shl(i as u32);
                }
                let mut col = 0;
                'search: loop {
                    if col + w > 32 {
                        x = -1;
                        y = -1;
                        break;
                    }
                    let mut start = 0;
                    while h + start < 33 {
                        let mut row = start;
                        loop {
                            if h + start <= row {
                                x = col * 8;
                                y = start * 8;
                                for r in start..start + h {
                                    rows[r as usize] |= mask;
                                }
                                break 'search;
                            }
                            if rows[row as usize] & mask != 0 {
                                break;
                            }
                            row += 1;
                        }
                        start = row + 1;
                    }
                    col += 1;
                    mask = mask.wrapping_shl(1);
                }
            }
            vm.set_prop(this, "TexRes", Value::Array(rows.into_iter().map(|r| Value::Int(r as i32)).collect()));
            args[2] = Value::Int(x);
            args[3] = Value::Int(y);
            Value::Void
        }
        "FreeRect" => {
            // URenderTargetMaterial::execFreeRect: X / 8 and Y / 8 clamped to [0, 31], cells
            // (W + 7) / 8 and (H + 7) / 8 clamped to the rest of the grid, bits cleared from TexRes.
            let cx = (n(0) / 8).clamp(0, 31);
            let cy = (n(1) / 8).clamp(0, 31);
            let w = ((n(2) + 7) / 8).clamp(0, 32 - cx);
            let h = ((n(3) + 7) / 8).clamp(0, 32 - cy);
            let mut mask: u32 = 0;
            for i in 0..w {
                mask |= 1u32.wrapping_shl(((i + cx) & 0x1f) as u32);
            }
            if let Value::Array(mut rows) = vm.prop(this, "TexRes") {
                for r in cy..cy + h {
                    if let Some(v) = rows.get_mut(r as usize) {
                        *v = Value::Int((v.as_int() as u32 & !mask) as i32);
                    }
                }
                vm.set_prop(this, "TexRes", Value::Array(rows));
            }
            Value::Void
        }
        // AActor::execIntersectWaterPlane: first water plane (ULevel+0x1188) crossed
        // by the segment, within its box in X and Y; the point written into Intersection.
        "IntersectWaterPlane" => {
            let (mut s, mut e) = (args[0].as_vector(), args[1].as_vector());
            let mut hit = None;
            if s[2] != e[2] {
                for (p, actor) in vm.water_planes.clone() {
                    if e[2] < s[2] {
                        std::mem::swap(&mut s, &mut e);
                    }
                    if s[2] <= p[2] && p[2] <= e[2] {
                        let t = (p[2] - s[2]) / (e[2] - s[2]);
                        let u = 1.0 - t;
                        let i = [e[0] * t + s[0] * u, e[1] * t + s[1] * u, u * s[2] + t * e[2]];
                        if (i[0] - p[0]).abs() < p[3] && (i[1] - p[1]).abs() < p[4] {
                            hit = Some((i, actor));
                            break;
                        }
                    }
                }
            }
            match hit {
                Some((i, actor)) => {
                    args[2] = vec(i);
                    Value::Object(Some(actor))
                }
                None => Value::Object(None),
            }
        }
        "CalcDrawOffset" => {
            // APawn::execCalcDrawOffset (Engine.dll): PlayerViewOffset of the held object,
            // × 90 / FovAngle of the controller, rotated by the controller's rotation. Controller of the
            // local player (LocalPlayerController: Player is a viewport, vm.local_player_controller): + EyeHeight and
            // APlayerController::WeaponBob(BobDamping); another controller:
            // + BaseEyeHeight. Without a controller: PlayerViewOffset rotated by the pawn, + BaseEyeHeight.
            let inv = args[0].as_object();
            let offset = inv.map(|i| vm.propr(i, "PlayerViewOffset").as_vector()).unwrap_or([0.0; 3]);
            let controller = vm.propr(this, "Controller").as_object().filter(|&c| vm.objects[c].is_some());
            // V.TransformVectorBy(GMath.UnitCoords * R): dot
            // products (x·Ax + y·Ay) + z·Az with the frame's axes.
            let turn = |r: [i32; 3], v: [f32; 3]| {
                use openxiii_assets::coords;
                coords::transform_vector(v, &coords::mul_rotator(&coords::UNIT, r))
            };
            let d = match controller {
                Some(c) => {
                    let fov = vm.propr(c, "FovAngle").as_float();
                    let scale = 90.0 / fov;
                    let rotation = vm.propr(c, "Rotation").as_rotator();
                    let mut d = turn(rotation, offset.map(|x| x * scale));
                    let local = vm.local_player_controller(c).is_some();
                    if local {
                        d[2] += vm.propr(this, "EyeHeight").as_float();
                        // WeaponBob: rotated ShakeOffset × 0.9 + BobDamping × WalkBob, and
                        // (0.45 + 0.55 × BobDamping) × WalkBob.Z in height.
                        let damping = inv.map_or(0.0, |i| vm.propr(i, "BobDamping").as_float());
                        let walk = vm.propr(c, "WalkBob").as_vector();
                        let shake = turn(rotation, vm.propr(c, "ShakeOffset").as_vector());
                        d[0] += shake[0] * 0.9 + damping * walk[0];
                        d[1] += shake[1] * 0.9 + damping * walk[1];
                        d[2] += shake[2] * 0.9 + (damping * 0.55 + 0.45) * walk[2];
                    } else {
                        d[2] += vm.propr(this, "BaseEyeHeight").as_float();
                    }
                    d
                }
                None => {
                    let mut d = turn(vm.propr(this, "Rotation").as_rotator(), offset);
                    d[2] += vm.propr(this, "BaseEyeHeight").as_float();
                    d
                }
            };
            vec(d)
        }
        // APlayerController::execAdjustAimForDisplay: see aim.rs.
        "AdjustAimForDisplay" => {
            let r = vm.adjust_aim_for_display(this, args[0].as_object(), args[1].as_vector());
            rot(r)
        }
        // APawn::execGetViewRotation: the controller's rotation, otherwise the pawn's
        // (like AActor::GetViewRotation).
        "GetViewRotation" => {
            match vm.propr(this, "Controller").as_object() {
                Some(c) if vm.objects[c].is_some() => vm.prop(c, "Rotation"),
                _ => vm.prop(this, "Rotation"),
            }
        }
        // AWeapon::execGetFireStart: without an Instigator, the Owner if it is a pawn (kept
        // in Instigator); the instigator's eyes + FireOffset in the X, Y, Z frame (only
        // FireOffset.X along X if bZoomed); zero vector without an instigator.
        "GetFireStart" => {
            let (x, y, z) = (args[0].as_vector(), args[1].as_vector(), args[2].as_vector());
            if vm.propr(this, "Instigator").as_object().is_none() {
                if let Some(o) = vm.propr(this, "Owner").as_object() {
                    let pawn = vm.is_a(o, "Pawn").then_some(o);
                    vm.set_prop(this, "Instigator", Value::Object(pawn));
                }
            }
            match vm.propr(this, "Instigator").as_object() {
                Some(p) => {
                    let l = vm.propr(p, "Location").as_vector();
                    let eye = vm.propr(p, "EyeHeight").as_float();
                    let off = vm.propr(this, "FireOffset").as_vector();
                    if vm.propr(this, "bZoomed").as_bool() {
                        vec([l[0] + x[0] * off[0], l[1] + x[1] * off[0], eye + l[2] + x[2] * off[0]])
                    } else {
                        vec([
                            l[0] + x[0] * off[0] + y[0] * off[1] + z[0] * off[2],
                            l[1] + x[1] * off[0] + y[1] * off[1] + z[1] * off[2],
                            eye + l[2] + x[2] * off[0] + y[2] * off[1] + z[2] * off[2],
                        ])
                    }
                }
                None => vec([0.0; 3]),
            }
        }
        "DirectionDuTir" => vec(vm.direction_shot(this)),
        "FindNewStakeOutDir" => {
            vm.find_new_stakeout_dir(this);
            Value::Void
        }
        "LineOfFireObstacle" => {
            // AIAController::execLineOfFireObstacle (XIDPawn.dll): trace from WeaponStartTrace towards
            // DirectionTir. Nothing, the enemy, its own pawn or the map: 0. A living non-hostile soldier:
            // it becomes Pote, 1 (a dead or hostile soldier: 0). Another actor: 2, unless one can
            // see through it (bCanSeeThrough): 0.
            // SingleLineCheck(Hit, this, DirectionTir, WeaponStartTrace, 0x4083 if the weapon fires
            // bullets (AmmoType.bInstantHit), 0x8083 otherwise, without extent).
            let start = vm.propr(this, "WeaponStartTrace").as_vector();
            let end = vm.propr(this, "DirectionTir").as_vector();
            let instant = vm
                .native_object(this, crate::NativeField::Pawn)
                .and_then(|p| vm.po(p, "Weapon"))
                .and_then(|w| vm.po(w, "AmmoType"))
                .is_some_and(|a| vm.propr(a, "bInstantHit").as_bool());
            let mut h = crate::physics::CheckResult::new(1.0);
            vm.single_line_check(&mut h, Some(this), end, start, if instant { 0x4083 } else { 0x8083 }, [0.0; 3]);
            let hit = h.actor;
            let pawn = vm.propr(this, "Pawn").as_object();
            let enemy = vm.propr(this, "Enemy").as_object();
            let soldier = vm.class("BaseSoldier");
            let result = match hit {
                None => 0,
                Some(a) if Some(a) == enemy || Some(a) == pawn || Some(a) == vm.level => 0,
                Some(a) if vm.objects[a].as_ref().is_some_and(|o| soldier.is_some_and(|s| vm.is_child_of(o.class, s))) => {
                    if vm.propr(a, "bIsDead").as_bool() {
                        0
                    } else if vm.alliance_level(this, Some(a)) >= 0 {
                        vm.set_prop(this, "Pote", Value::Object(Some(a)));
                        1
                    } else {
                        0
                    }
                }
                // Another actor: (~(+0x34 >> 12)) & 2, 2 unless it has bCanSeeThrough.
                Some(a) => {
                    if vm.propr(a, "bCanSeeThrough").as_bool() {
                        0
                    } else {
                        2
                    }
                }
            };
            Value::Int(result)
        }
        "LigneVisee" => {
            // AIAController::execLigneVisee (XIDPawn.dll): false without a weapon; otherwise start =
            // TraceStart + eyes + FireOffset (frame of the controller's rotation), +16 in front for a bullet
            // weapon; true if neither the map nor a Mover cuts the line up to TraceEnd (pawns do not).
            let (end, start) = (args[0].as_vector(), args[1].as_vector());
            let pawn = vm.propr(this, "Pawn").as_object();
            let weapon = pawn.and_then(|p| vm.propr(p, "Weapon").as_object());
            match (pawn, weapon) {
                (Some(p), Some(w)) => {
                    let [x, y, z] = crate::ai_natives::axes(vm.propr(this, "Rotation").as_rotator());
                    let off = vm.propr(w, "FireOffset").as_vector();
                    let eye = vm.propr(p, "BaseEyeHeight").as_float();
                    // ((TraceStart (+ BaseEyeHeight in Z) + X·Fx) + Y·Fy) + Z·Fz.
                    let mut s = [0, 1, 2].map(|k| ((start[k] + if k == 2 { eye } else { 0.0 }) + x[k] * off[0] + y[k] * off[1]) + z[k] * off[2]);
                    let instant = vm.propr(w, "AmmoType").as_object().is_some_and(|a| vm.propr(a, "bInstantHit").as_bool());
                    if instant {
                        s = [0, 1, 2].map(|k| 16.0 * x[k] + s[k]);
                    }
                    // SingleLineCheck(Hit, this, TraceEnd, start, 0x286, without extent): true without a contact.
                    let mut h = crate::physics::CheckResult::new(1.0);
                    vm.single_line_check(&mut h, Some(this), end, s, 0x286, [0.0; 3]);
                    Value::Bool(h.actor.is_none())
                }
                _ => Value::Bool(false),
            }
        }
        // ALevelInfo::execGetPlateForme: GSys+0x7C, [Core.System] PlateForm of the game's
        // ini (0 PC, 1 PS2, 2 Xbox, 3 GameCube according to the scripts; 0 in XIII.ini).
        "GetPlateForme" => {
            let v = vm.config.as_mut().and_then(|c| c.get_string("Core.System", "PlateForm", "System"));
            Value::Int(v.map_or(0, |v| crate::config::atoi(&v.chars().collect::<Vec<_>>(), 0)))
        }
        // AActor::execGetLastTraceBone: set by Trace / TraceActors.
        "GetLastTraceBone" => Value::Name(crate::value::NameStruct::from(vm.last_trace_bone.clone())),
        // AProjector::execAbandonProjector: lifetime of the projector's render
        // (RenderInfo + 0xC = now + 1, the argument is ignored), then Abandon (vt 0x148).
        "AbandonProjector" => {
            vm.projector_abandon(this);
            Value::Void
        }
        // AProjector::execAttachProjector: Attach (vt 0x140).
        "AttachProjector" => {
            vm.projector_attach(this);
            Value::Void
        }
        // AProjector::execDetachProjector: Detach(Force) (vt 0x144).
        "DetachProjector" => {
            let force = args.first().is_some_and(|v| v.as_bool());
            vm.projector_detach(this, force);
            Value::Void
        }
        // UForceFeedbackController::execIsForceFeedbackEnable: false without
        // UForceFeedbackManager::Instance (the case of the original PC version), otherwise vtable +0x7C.
        "IsForceFeedbackEnable" => {
            let client = vm.coop_feedback_client(this);
            Value::Bool(vm.force_feedback.as_ref().is_some_and(|m| m.is_enable(client) != 0))
        }
        "MakeNoise" => {
            let loudness = f(0);
            vm.make_noise(this, loudness);
            Value::Void
        }
        // AIAController::execAllianceLevel (XIDPawn.dll).
        "AllianceLevel" => Value::Int(vm.alliance_level(this, args[0].as_object())),
        "SetEnemy" => {
            // AIAController::execSetEnemy (XIDPawn.dll): refuses itself, None and the dead;
            // true if it is already the enemy; otherwise, if the pawn is hostile (AllianceLevel < 0) and closer
            // than the current enemy (or if there is none), it becomes the enemy: Focus,
            // LastSeenTime, LastSeeingPos, LastSeenPos, then the EnemyAcquired event.
            let pawn = vm.propr(this, "Pawn").as_object();
            let candidate = args[0].as_object();
            let accepted = match candidate {
                Some(c) if Some(c) != pawn && vm.objects[c].is_some() && !vm.propr(c, "bIsDead").as_bool() => {
                    if vm.propr(this, "Enemy").as_object() == Some(c) {
                        true
                    } else if vm.alliance_level(this, Some(c)) < 0 {
                        let here = pawn.map(|p| vm.propr(p, "Location").as_vector()).unwrap_or([0.0; 3]);
                        // (Location − Pawn.Location): SizeSquared,
                        // (X² + Y²) + Z², without a root.
                        let dist = |vm: &Vm, o: ObjId| crate::ai_natives::size2(crate::ai_natives::vect_minus(vm.propr(o, "Location").as_vector(), here));
                        let closer = match vm.propr(this, "Enemy").as_object() {
                            Some(old) if vm.objects[old].is_some() => dist(vm, old) > dist(vm, c),
                            _ => true,
                        };
                        if closer {
                            let now = vm.level.map(|l| vm.prop(l, "TimeSeconds")).unwrap_or(Value::Float(0.0));
                            vm.set_prop(this, "Enemy", Value::Object(Some(c)));
                            vm.set_prop(this, "LastSeenTime", now);
                            vm.set_prop(this, "LastSeeingPos", vec(here));
                            let seen = vm.prop(c, "Location");
                            vm.set_prop(this, "LastSeenPos", seen);
                            vm.set_prop(this, "Focus", Value::Object(Some(c)));
                            // AIAController::eventEnemyAcquired (XIDPawn.dll).
                            vm.event_pub(this, "EnemyAcquired", vec![]);
                        }
                        closer
                    } else {
                        false
                    }
                }
                _ => false,
            };
            Value::Bool(accepted)
        }
        "HalteAuFeu" => {
            // AIAController::execHalteAuFeu (XIDPawn.dll): if the soldier exists, bTire and bFire at
            // zero, timers Timer and Timer2 stopped, torso aim given back (bEnableSpineControl). The
            // SetBoneDirection targets the controller's instance with the soldier's Mesh,
            // even if the controller's Mesh property is null.
            if let Some(soldier) = vm.propr(this, "BaseS").as_object() {
                vm.set_prop(this, "bTire", Value::Bool(false));
                vm.set_prop(this, "bFire", Value::Byte(0));
                // (+0x48 / +0x4C / +0x50 / +0x54 at zero, bTimerLoop and bTimer2Loop cleared.)
                for p in ["TimerRate", "TimerCounter", "Timer2Rate", "Timer2Counter"] {
                    vm.set_prop(this, p, Value::Float(0.0));
                }
                vm.set_prop(this, "bTimerLoop", Value::Bool(false));
                vm.set_prop(this, "bTimer2Loop", Value::Bool(false));
                vm.set_prop(soldier, "bEnableSpineControl", Value::Bool(false));
                if let Value::Asset(mesh) = vm.prop(soldier, "Mesh") {
                    if vm.mesh_info(&mesh).is_some() {
                        if let Some(inst) = vm.mesh_instance_from(this, &mesh) {
                            let info = inst.info.clone();
                            crate::skel::direction_os(&mut inst.pose.mesh_dirs, &info, "X Spine", [0; 3], [0.0; 3], 0.0);
                        }
                    }
                }
            }
            Value::Void
        }
        // AActor::execSetTimer / execSetTimer2, AController::execSetTimer3
        // counter at zero, TimerRate = NewTimerRate (even zero or negative), bTimerLoop.
        "SetTimer" | "SetTimer2" | "SetTimer3" => {
            let n = match name {
                "SetTimer2" => "2",
                "SetTimer3" => "3",
                _ => "",
            };
            vm.set_prop(this, &format!("Timer{n}Counter"), Value::Float(0.0));
            vm.set_prop(this, &format!("Timer{n}Rate"), Value::Float(f(0)));
            vm.set_prop(this, &format!("bTimer{n}Loop"), Value::Bool(args.get(1).is_some_and(|b| b.as_bool())));
            Value::Void
        }
        // APawn::execSpineYawControl: bSpineYaw, MaxSpineYaw, SpineYawRotationSpeed.
        "SpineYawControl" => {
            vm.set_prop(this, "bSpineYaw", Value::Bool(args[0].as_bool()));
            vm.set_prop(this, "MaxSpineYaw", Value::Int(args[1].as_int()));
            vm.set_prop(this, "SpineYawRotationSpeed", Value::Float(args[2].as_float()));
            Value::Void
        }
        // AActor::execRecomputeBoundingVolume: render box of the skeletal mesh.
        "RecomputeBoundingVolume" => {
            vm.recompute_bounding_volume(this, args.first().is_some_and(|v| v.as_bool()));
            Value::Void
        }
        // AActor::execRefreshDisplaying: XLevel->RefreshDynDisp except on a dedicated server.
        "RefreshDisplaying" => {
            if vm.level.is_none_or(|l| vm.propr(l, "NetMode").as_byte() != 1) {
                vm.refresh_dyn_disp(this);
            }
            Value::Void
        }
        // ALevelInfo::exec…RndCubeSpr: weather of FD3DRenderInterface.
        "InitRndCubeSpr" | "SetRndCubeSprState" | "SetRndCubeSprSpeed" | "ChangeRndCubeSprProp" | "AddRndCubeSprExclude" | "SetRndCubeSprSize" => {
            vm.weather_native(name, args, |i| supplied.get(i) == Some(&true)).unwrap_or(Value::Void)
        }
        // AActor::execReplaceATextureByAnOther: in all loaded objects, the
        // texture Src replaced by Dest: Materials[i].Material of the StaticMeshes, texture of the
        // emitters (+0x1D4), and Diffuse / Opacity / Specular… of the Shaders. No effect on the
        // logic; the VM keeps the list for display.
        "ReplaceATextureByAnOther" => {
            let (src, dst) = (crate::canvas::material_path(&args[0]), crate::canvas::material_path(&args[1]));
            if !src.is_empty() {
                vm.texture_replacements.push((src, dst));
            }
            Value::Void
        }
        "GetBoundingBox" => {
            // AActor::execGetBoundingBox: render box through LocalToWorld (IsValid
            // set by FBox += as in the engine).
            let (lo, hi) = vm.render_bounding_box(this, false);
            let mut b = vm.zero(&crate::types::Type::Struct("Object.Box".into()), 1);
            if let Value::Struct(_, fields) = &mut b {
                if fields.len() >= 3 {
                    fields[0] = Value::vector(lo[0], lo[1], lo[2]);
                    fields[1] = Value::vector(hi[0], hi[1], hi[2]);
                    fields[2] = Value::Byte(1);
                }
            }
            b
        }
        // AActor::execSetDrawScale3D: AActor::SetDrawScale3D, removed then
        // put back into the collision table if it has bCollideActors, bNetDirty.
        "SetDrawScale3D" => {
            vm.set_prop(this, "DrawScale3D", args[0].clone());
            if vm.propr(this, "bCollideActors").as_bool() {
                vm.refresh_collider(this);
            }
            vm.set_prop(this, "bNetDirty", Value::Bool(true));
            Value::Void
        }
        // AActor::execAttachToBone: the attachment takes the bone then the actor as its base
        // (SetBase with a floor (0, 0, 1)); false if the mesh is not skeletal, the bone
        // unknown (message) or the attachment missing.
        "AttachToBone" => {
            let bone = args[1].as_name();
            let Some(info) = vm.mesh_instance(this).map(|i| i.info.clone()) else {
                return Ok(Value::Bool(false));
            };
            match args[0].as_object().filter(|&o| vm.objects.get(o).is_some_and(|x| x.is_some())) {
                Some(attachment) if crate::skel::match_ref_bone(&info, &bone) >= 0 => {
                    vm.set_prop(attachment, "AttachmentBone", Value::Name(crate::value::NameStruct::from(bone)));
                    vm.set_base_full(attachment, Some(this), [0.0, 0.0, 1.0], true);
                    Value::Bool(true)
                }
                _ => {
                    let name = vm.objects[this].as_ref().map(|o| o.name.clone()).unwrap_or_default();
                    vm.log.push(format!("Log: AttachToBone: No bone named [{bone}] found in actor {name}'s skeleton."));
                    Value::Bool(false)
                }
            }
        }
        // AActor::execDetachFromBone: base removed (floor (0, 0, 1)) then bone cleared.
        "DetachFromBone" => {
            if vm.mesh_instance(this).is_none() {
                return Ok(Value::Bool(false));
            }
            let Some(attachment) = args[0].as_object().filter(|&o| vm.objects.get(o).is_some_and(|x| x.is_some())) else {
                return Ok(Value::Bool(false));
            };
            vm.set_base_full(attachment, None, [0.0, 0.0, 1.0], true);
            vm.set_prop(attachment, "AttachmentBone", Value::Name(crate::value::NameStruct::from(String::new())));
            Value::Bool(true)
        }
        // UGUIController::execInitController (GUI.dll): only loads the class
        // IpDrv.ClientBeaconReceiver.
        "InitController" => {
            let events = vm.lo_run(|vm, lo| {
                vm.lo_load(lo, "IpDrv.ClientBeaconReceiver", Some("Class"));
            });
            vm.apply_post_loads(&events, None);
            Value::Str(String::new())
        },
        // UXboxLiveManager: no Xbox Live on PC. IsLoggedIn and
        // GetCurrentUser go through the Engine's UXboxLive when Init has kept it (xbox_live.rs).
        "IsLoggedIn" => Value::Bool(vm.xbox_live.engine && vm.xbox_live.is_logged_in()),
        "IsLoggedInTwice" => Value::Bool(false),
        "GetCurrentUser" => Value::Str(if vm.xbox_live.engine { vm.xbox_live.friend_join_get_url() } else { String::new() }),
        // APlayerController::execConsoleCommand: the command goes through the player
        // (UViewport::Exec, UPlayer::Exec); AActor::execConsoleCommand and the
        // engine's commands (open, set…) are not reimplemented yet.
        // APlayerController::execConsoleCommand: through the player's window
        // (UWindowsViewport::Exec), otherwise through the engine; UInteraction::execConsoleCommand
        // through the first player's window (UInteractionMaster::Exec), true if taken;
        // AActor::execConsoleCommand: UGameEngine::Exec. The written text is returned.
        "ConsoleCommand" => {
            // Flag set even without a Player.
            let from_pc = vm.is_a(this, "PlayerController");
            if from_pc { vm.command_from_pc = true; }
            let result = if vm.is_a(this, "Interaction") {
                // Without a Master: a warning (GWarn), nothing run. Otherwise
                // UInteractionMaster::Exec: Client->Viewports(0)->Exec.
                if vm.propr(this, "Master").as_object().is_none() {
                    // GWarn->Logf(Fmt): NAME_Log event (760), not "Warning:".
                    vm.log.push("Log: Attempt to execute a console command through an Interaction that is without a master".into());
                    Value::Bool(false)
                } else {
                    // Lockstep co-op: the interaction's own player (every player's interactions run
                    // on every machine), not this machine's.
                    let own = vm.propr(this, "ViewportOwner").as_object().and_then(|p| vm.propr(p, "Actor").as_object()).filter(|_| vm.lockstep.is_some());
                    let pc = own.or_else(|| vm.local_viewport().and_then(|p| vm.propr(p, "Actor").as_object()));
                    Value::Bool(pc.is_some_and(|pc| vm.console_command_out(pc, &s(0)).0))
                }
            } else if let Some(player) = vm.propr(this, "Player").as_object().filter(|&p| vm.is_a(this, "PlayerController") && vm.net_is_remote_player(p)) {
                // Controller of a client on the host: Player = the connection (UNetConnection::Exec,
                // then UPlayer::Exec).
                let cmd = s(0);
                match vm.net_connection_exec(player, &cmd) {
                    Some(text) => Value::Str(text),
                    None => Value::Str(vm.player_exec_out(this, &cmd).1),
                }
            } else if vm.is_a(this, "PlayerController") && vm.propr(this, "Player").as_object().is_some() {
                Value::Str(vm.console_command_from_pc(this, &s(0)).1)
            } else {
                let mut out = String::new();
                vm.engine_exec(&s(0), &mut out);
                Value::Str(out)
            };
            if from_pc { vm.command_from_pc = false; }
            result
        }
        // APlayerController::execUpdateURL: option added to the engine's LastURL;
        // bSaveDefault: FURL::SaveURLConfig("DefaultPlayer", Option, "User").
        "UpdateURL" => {
            let (key, value) = (s(0), s(1));
            vm.url.add_option(&format!("{key}={value}"));
            if args.get(2).is_some_and(|b| b.as_bool()) {
                for (k, v) in vm.url.save_url_config(&key) {
                    if let Some(c) = vm.config.as_mut() {
                        c.set_string("DefaultPlayer", &k, &v, "User");
                    }
                }
            }
            Value::Void
        }
        // execGetDefaultURL: FURL::LoadURLConfig("DefaultPlayer", "User") then
        // GetOption(Option + "=", "").
        "GetDefaultURL" => {
            let mut url = crate::travel::Url::default();
            let pairs = vm.config.as_mut().map(|c| c.section_pairs("DefaultPlayer", "User")).unwrap_or_default();
            url.load_url_config(&pairs);
            Value::Str(url.get_option(&format!("{}=", s(0))).unwrap_or("").to_string())
        }
        // ALevelInfo::execGetAddressURL: appSprintf("%s:%i", XLevel->URL.Host (+0x50),
        // XLevel->URL.Port (+0x5C)); without its own port the URL has the default port, [URL] Port.
        "GetAddressURL" => {
            let port = match vm.url.port {
                Some(p) => p,
                None => vm.url_default_port(),
            };
            Value::Str(format!("{}:{port}", vm.url.host))
        }
        // ALevelInfo::execGetLocalURL: FURL::String of the level's URL.
        "GetLocalURL" => Value::Str(vm.url.string()),
        // AActor::execGetURLMap: the map of the current URL.
        "GetURLMap" => Value::Str(vm.url.map.clone()),
        // APlayerController::execClientTravel: with a Player, PreClientTravel then
        // UGameEngine::SetClientTravel, the travel is done by the engine at the end of
        // the frame. ClientTravel is static (0x800): ProcessRemoteFunction never sends it, and the
        // Player of a remote client (UTcpipConnection) makes CastChecked<UViewport> fail
        // (GError: fatal error of the original game, ProcessServerTravel with clients).
        "ClientTravel" => {
            let player = vm.propr(this, "Player").as_object();
            let remote = player.is_some_and(|p| vm.ipdrv.net_driver.as_ref().is_some_and(|d| d.conns.iter().any(|c| c.player == Some(p))));
            if let Some(p) = player {
                // APlayerController::eventPreClientTravel, sent before SetClientTravel even for a remote Player.
                let (type_, items) = (args.get(1).map_or(0, |v| v.as_byte()), args.get(2).is_some_and(|v| v.as_bool()));
                if let Some(f) = vm.find_function(this, "PreClientTravel", false) {
                    vm.call(f, this, vec![])?;
                }
                if remote {
                    // Deliberate difference (net_send_static_to_client): the RPC goes to the client instead of the original's GError
                    // "Cast of TcpipConnection … to Viewport failed", which crashed here.
                    let sent = match vm.find_function(this, "ClientTravel", false) {
                        // Parameters (URL, TravelType, bItems) = the function's first locals.
                        Some(f) => vm.net_send_static_to_client(f, this, &args.to_vec()),
                        None => false,
                    };
                    if !sent {
                        let message = format!("Cast of TcpipConnection {} to Viewport failed", vm.path_name(p));
                        vm.log.push(format!("Critical: {message}"));
                        vm.warn(format!("{message} (fatal error in the original game)"));
                    }
                } else {
                    vm.engine_set_client_travel(s(0), type_, items);
                }
            }
            Value::Void
        }
        // AActor::execTrace: ULevel::SingleLineCheck, like MoveActor. Flags:
        // TRACE_World (0x86), plus Pawns | Volumes | Others | OnlyProjActor (0x39) if bTraceActors
        // (bCollideActors by default), TRACE_Material if Material is requested, and AdditionalTraceType;
        // DiscardedHitMask receives the reasons for the discarded actors. Without a contact, HitLocation and
        // HitNormal zero.
        "Trace" => {
            let end = args[2].as_vector();
            let start = if supplied.get(3) == Some(&true) { args[3].as_vector() } else { vm.propr(this, "Location").as_vector() };
            let trace_actors = if supplied.get(4) == Some(&true) { args[4].as_bool() } else { vm.propr(this, "bCollideActors").as_bool() };
            let extent = if supplied.get(5) == Some(&true) { args[5].as_vector() } else { [0.0; 3] };
            let want_material = supplied.get(6) == Some(&true);
            let additional = if supplied.get(7) == Some(&true) { args[7].as_int() as u32 } else { 0 };
            let mut flags = if trace_actors { 0xbf } else { 0x86 };
            // The Material pointer (the parameter's or a local variable) is never null
            // TRACE_Material (0x1000) is always requested.
            flags |= crate::physics::TRACE_MATERIAL;
            let mut hit = crate::physics::CheckResult::new(1.0);
            let mut discarded = 0u32;
            vm.single_line_check_masked(&mut hit, Some(this), end, start, flags | additional, extent, &mut discarded);
            // ULevel::SingleLineCheck tests the bones of each actor whose cylinder is hit.
            vm.pose_candidates_os();
            vm.skeletal_bone_check(&mut hit, end, start, flags | additional);
            if hit.actor.is_some() {
                args[0] = vec(hit.location);
                args[1] = vec(hit.normal);
            } else {
                // Without a contact, execTrace still copies Hit.Location and Hit.Normal
                // from SingleLineCheck's local result, of which only Time and Actor are set
                // (with Material): Location, Normal, Primitive, Item and Next keep what
                // the stack contained at that depth, the last contact of a previous call
                // if it has not been rewritten since. Zero here; the stack leftovers are not reproduced.
                args[0] = vec([0.0; 3]);
                args[1] = vec([0.0; 3]);
            }
            // Material of the surface hit (sound and impact marks). Hit.Material->GetAllMaterialSounds()
            // (vtable + 0x88) as soon as Hit.Material is non-null, whether Material is passed or not: the
            // output pointer is GPropAddr or a local variable. The footstep sounds
            // of a Shader are thus filled in at the first shot that hits it.
            let material = match hit.material.and_then(|t| vm.collision_materials.get(t as usize)).cloned() {
                Some(m) if !m.is_empty() => {
                    vm.material_fill_sounds(&m);
                    Value::Asset(m)
                }
                _ => Value::Object(None),
            };
            if want_material && args.len() > 6 {
                args[6] = material;
            }
            if supplied.get(8) == Some(&true) && args.len() > 8 {
                args[8] = Value::Int(discarded as i32);
            }
            // TRACETYPE_RequestBones: the bone hit (NAME_None without a bone), read by GetLastTraceBone.
            if additional & crate::bone_hit::TRACE_REQUEST_BONES != 0 {
                vm.last_trace_bone = vm.trace_bone_name(&hit).unwrap_or_default();
            }
            Value::Object(hit.actor)
        }
        // AActor::execFastTrace: SingleLineCheck without extent, TRACE_World |
        // TRACE_StopAtFirstHit (0x286) and AdditionalTraceType; true if nothing is hit.
        "FastTrace" => {
            let end = args[0].as_vector();
            let start = if supplied.get(1) == Some(&true) { args[1].as_vector() } else { vm.propr(this, "Location").as_vector() };
            let additional = if supplied.get(2) == Some(&true) { args[2].as_int() as u32 } else { 0 };
            let mut hit = crate::physics::CheckResult::new(1.0);
            let mut discarded = 0u32;
            vm.single_line_check_masked(&mut hit, Some(this), end, start, additional | 0x286, [0.0; 3], &mut discarded);
            if supplied.get(3) == Some(&true) && args.len() > 3 {
                args[3] = Value::Int(discarded as i32);
            }
            Value::Bool(hit.actor.is_none())
        }
        // AVolume::execEncompasses: Encompasses(Other.Location).
        "Encompasses" => Value::Bool(args[0].as_object().is_some_and(|o| vm.encompasses(this, vm.propr(o, "Location").as_vector()))),
        // AActor::execSaveAtCheckpoint: USaveGameDevice::SaveAtCheckpoint(Portal, Description).
        "SaveAtCheckpoint" => Value::Bool(vm.save_at_checkpoint(&s(0), &s(1))),
        // APawn::execIsPlayerPawn: Controller is a PlayerController.
        "IsPlayerPawn" => {
            let controller = vm.propr(this, "Controller").as_object().filter(|&c| vm.objects.get(c).is_some_and(|o| o.is_some()));
            Value::Bool(controller.is_some_and(|c| vm.is_a(c, "PlayerController")))
        }
        // APawn::execIsHumanControlled: Controller.bIsPlayer (also true for a bot).
        "IsHumanControlled" => {
            let controller = vm.propr(this, "Controller").as_object().filter(|&c| vm.objects.get(c).is_some_and(|o| o.is_some()));
            Value::Bool(controller.is_some_and(|c| vm.propr(c, "bIsPlayer").as_bool()))
        }
        // APawn::execIsLocallyControlled: always true outside networking (NetMode 0);
        // otherwise Controller->LocalPlayerController (vt 0x150: a PlayerController whose Player is a
        // UViewport; vm.local_player_controller).
        "IsLocallyControlled" => {
            let standalone = vm.level.is_none_or(|l| vm.propr(l, "NetMode").as_byte() == 0);
            let controller = vm.propr(this, "Controller").as_object().filter(|&c| vm.objects.get(c).is_some_and(|o| o.is_some()));
            Value::Bool(standalone || controller.is_some_and(|c| vm.local_player_controller(c).is_some()))
        }
        // APawn::execEyePosition: (0, 0, EyeHeight); ABaseSoldier::execEyePosition
        // (0, 0, BaseEyeHeight).
        "EyePosition" => {
            let [x, y, z] = vm.eye_position(this);
            Value::vector(x, y, z)
        }
        // ALevelInfo::execSetOnlyPostRender: UViewport::SetOnlyPostRender of the player's
        // viewport.
        "SetOnlyPostRender" => {
            vm.only_post_render = args[0].as_bool();
            Value::Void
        }
        // AActor::execSetDrawScale: AActor::SetDrawScale, like SetDrawScale3D.
        "SetDrawScale" => {
            vm.set_prop(this, "DrawScale", args[0].clone());
            if vm.propr(this, "bCollideActors").as_bool() {
                vm.refresh_collider(this);
            }
            vm.set_prop(this, "bNetDirty", Value::Bool(true));
            Value::Void
        }
        // AActor::execSetRelativeLocation: attached to a bone, the relative position is
        // recorded; otherwise, with a base, FarMoveActor (attachedMove) to the relative position in
        // the base's frame, then RelativeLocation = Location − Base.Location. False without a base.
        "SetRelativeLocation" => {
            let rel = args[0].as_vector();
            if !vm.propr(this, "AttachmentBone").as_name().is_empty() {
                vm.set_prop(this, "RelativeLocation", args[0].clone());
                return Ok(Value::Bool(true));
            }
            let Some(base) = vm.propr(this, "Base").as_object() else {
                return Ok(Value::Bool(false));
            };
            let [x, y, z] = crate::ai_natives::axes(vm.propr(base, "Rotation").as_rotator());
            let b = vm.propr(base, "Location").as_vector();
            let dest = [0, 1, 2].map(|k| x[k] * rel[0] + b[k] + y[k] * rel[1] + z[k] * rel[2]);
            let moved = vm.far_move_actor(this, dest, false, false, true);
            if let Some(base) = vm.propr(this, "Base").as_object() {
                let (l, b) = (vm.propr(this, "Location").as_vector(), vm.propr(base, "Location").as_vector());
                vm.set_prop(this, "RelativeLocation", Value::vector(l[0] - b[0], l[1] - b[1], l[2] - b[2]));
            }
            Value::Bool(moved)
        }
        // AActor::execSetRelativeRotation: attached to a bone, the relative rotation is
        // recorded; otherwise MoveActor without movement towards the given rotation, composed with that of the
        // base if there is one.
        "SetRelativeRotation" => {
            let mut rot = args[0].as_rotator();
            if !vm.propr(this, "AttachmentBone").as_name().is_empty() {
                vm.set_prop(this, "RelativeRotation", args[0].clone());
                return Ok(Value::Bool(true));
            }
            if let Some(base) = vm.propr(this, "Base").as_object() {
                let b = crate::ai_natives::axes(vm.propr(base, "Rotation").as_rotator());
                let r = crate::ai_natives::axes(rot);
                let world = r.map(|a| [0, 1, 2].map(|k| a[0] * b[0][k] + a[1] * b[1][k] + a[2] * b[2][k]));
                rot = crate::world::ortho_rotation(world);
            }
            let mut hit = crate::physics::CheckResult::new(1.0);
            Value::Bool(vm.move_actor(this, [0.0; 3], rot, &mut hit, false, false, false, false))
        }
        "WantsSmoothedView" => {
            // AController::execWantsSmoothedView: pawn walking (or PHYS_Spider)
            // that has not just landed.
            let smoothed =
                vm.propr(this, "Pawn").as_object().is_some_and(|p| matches!(vm.propr(p, "Physics").as_byte(), 1 | 9) && !vm.propr(p, "bJustLanded").as_bool());
            Value::Bool(smoothed)
        }
        "SetViewTarget" => {
            // APlayerController::execSetViewTarget: None targets the pawn (if it is not
            // destroyed), otherwise the controller itself.
            let target = args[0]
                .as_object()
                .or_else(|| vm.propr(this, "Pawn").as_object().filter(|&p| vm.objects[p].is_some() && !vm.propr(p, "bDeleteMe").as_bool()))
                .unwrap_or(this);
            vm.set_prop(this, "ViewTarget", Value::Object(Some(target)));
            Value::Void
        }
        "Move" => {
            // AActor::execMove: ULevel::MoveActor with the current rotation.
            let r = vm.propr(this, "Rotation").as_rotator();
            let mut hit = crate::physics::CheckResult::new(1.0);
            Value::Bool(vm.move_actor(this, args[0].as_vector(), r, &mut hit, false, false, false, false))
        }
        "MoveSmooth" => {
            // AActor::execMoveSmooth.
            vm.set_prop(this, "bJustTeleported", Value::Bool(false));
            Value::Bool(vm.move_smooth(this, args[0].as_vector()))
        }
        "AutonomousPhysics" => {
            // AActor::execAutonomousPhysics: acceleration rounded to the tenth, then
            // performPhysics (pawns).
            let a = vm.propr(this, "Acceleration").as_vector();
            let q = a.map(|x| ((x * 10.0) as i32) as f32 * 0.1);
            vm.set_prop(this, "Acceleration", Value::vector(q[0], q[1], q[2]));
            if vm.propr(this, "Physics").as_byte() != 0 && vm.is_a(this, "Pawn") {
                vm.pawn_perform_physics(this, args[0].as_float());
            }
            Value::Void
        }
        "MakeColor" => {
            // Order of Color's fields in XIII: B, G, R, A.
            let (r, g, b) = (args[0].as_byte(), args[1].as_byte(), args[2].as_byte());
            let a = if supplied.get(3) == Some(&true) { args[3].as_byte() } else { 255 };
            Value::Struct("object.color".into(), vec![Value::Byte(b), Value::Byte(g), Value::Byte(r), Value::Byte(a)].into())
        }
        // UObject::execOrthoRotation: FCoords(0, X, Y, Z).OrthoRotation().
        "OrthoRotation" => rot(crate::world::ortho_rotation([args[0].as_vector(), args[1].as_vector(), args[2].as_vector()])),
        // APawn::execFindInventoryType: exact class; execFindInventoryKind
        // name of the class.
        "FindInventoryType" | "FindInventoryKind" => {
            let wanted_class = args[0].as_class();
            let wanted_name = args[0].as_name();
            let mut item = vm.propr(this, "Inventory").as_object();
            let mut found = None;
            let mut guard = 0;
            while let (Some(i), true) = (item, guard < 1000) {
                guard += 1;
                let class = vm.objects[i].as_ref().map(|o| o.class);
                let hit = match (class, wanted_class) {
                    (Some(c), Some(w)) => c == w,
                    (Some(c), None) => vm.classes[c].name.eq_ignore_ascii_case(&wanted_name),
                    _ => false,
                };
                if hit {
                    found = Some(i);
                    break;
                }
                item = vm.propr(i, "Inventory").as_object();
            }
            Value::Object(found)
        }
        "HasAmmo" => {
            // AAmmunition::execHasAmmo: AmmoAmount > 0. AWeapon::execHasAmmo
            // without an AmmoType, false; dual weapon (MySlave or SlaveOf), more
            // ammunition than the other weapon's ReloadCount; otherwise AmmoAmount > 0.
            if vm.prop_slot(this, "AmmoAmount").is_ok() {
                Value::Bool(vm.propr(this, "AmmoAmount").as_int() > 0)
            } else {
                match vm.propr(this, "AmmoType").as_object() {
                    None => Value::Bool(false),
                    Some(a) => {
                        let amount = vm.propr(a, "AmmoAmount").as_int();
                        let other = vm.propr(this, "MySlave").as_object().or_else(|| vm.propr(this, "SlaveOf").as_object());
                        match other {
                            // 32-bit SUB, then TEST and SETG.
                            Some(o) => Value::Bool(amount.wrapping_sub(vm.propr(o, "ReloadCount").as_byte() as i32) > 0),
                            None => Value::Bool(amount > 0),
                        }
                    }
                }
            }
        }
        // AActor::execSpawn then ULevel::SpawnActor.
        "Spawn" => match args[0].as_class() {
            Some(class) => {
                let location = if supplied.get(3) == Some(&true) { args[3].clone() } else { vm.prop(this, "Location") };
                let rotation = if supplied.get(4) == Some(&true) { args[4].clone() } else { vm.prop(this, "Rotation") };
                // AActor::execSpawn: SpawnOwner is None when it is omitted.
                let owner = if supplied.get(1) == Some(&true) { args[1].clone() } else { Value::Object(None) };
                // Spawned->Tag = SpawnTag, NAME_None when it is omitted (the Tag from the class name
                // set by SpawnActor is overwritten).
                let tag = if supplied.get(2) == Some(&true) { args[2].clone() } else { Value::Name(crate::value::NameStruct::from(String::new())) };
                // Like AActor::execSpawn: the new actor inherits the Instigator of the one that creates it.
                let instigator = vm.prop(this, "Instigator");
                let spawned = vm.spawn_actor(class, owner, None, location, rotation, instigator, false)?;
                if let Some(a) = spawned.filter(|&a| vm.objects.get(a).is_some_and(|o| o.is_some())) {
                    vm.set_prop(a, "Tag", tag);
                }
                Value::Object(spawned)
            }
            None => Value::Object(None),
        },
        // AActor::execGetWaveDuration / execWaveHasPosition:
        // GetLevel()->Engine->Audio, vtable +0x94 / +0x90 with the name; without audio, −1.0
        // (0xBF800000) and false.
        "GetWaveDuration" => {
            let d = vm.audio.as_ref().map_or(-1.0, |a| a.wave_duration(&s(0)));
            Value::Float(d)
        }
        "WaveHasPosition" => {
            let r = vm.audio.as_ref().is_some_and(|a| a.wave_has_position(&s(0)));
            Value::Bool(r)
        }
        // AActor::execDestroy: ULevel::DestroyActor (see `actor_iter`).
        "Destroy" => Value::Bool(vm.destroy_actor(this)?),
        // Linked lists kept by the engine in LevelInfo. AController::execAddController
        // / APawn::execAddPawnToList: added at the head without a test.
        "AddController" | "AddPawnToList" => {
            let list = if name == "AddController" { LevelActorList::Controllers } else { LevelActorList::Pawns };
            if let Some(level) = vm.level {
                let head = vm.prop(level, list.head());
                list.set_next(vm, this, head);
                vm.set_prop(level, list.head(), Value::Object(Some(this)));
            }
            Value::Void
        }
        // AController::execRemoveController / APawn::execRemovePawnFromList
        // removed from the head or from the link before it; its own link stays.
        "RemoveController" | "RemovePawnFromList" => {
            let list = if name == "RemoveController" { LevelActorList::Controllers } else { LevelActorList::Pawns };
            if let Some(level) = vm.level {
                let following = match list {
                    LevelActorList::Controllers => vm.native_value(this, crate::NativeField::NextController),
                    LevelActorList::Pawns => vm.prop(this, "nextPawn"),
                };
                if vm.propr(level, list.head()).as_object() == Some(this) {
                    vm.set_prop(level, list.head(), following);
                } else {
                    let mut current = vm.propr(level, list.head()).as_object();
                    let mut guard = 0;
                    while let (Some(c), true) = (current, guard < 100_000) {
                        guard += 1;
                        let n = list.next(vm, c).as_object();
                        if n == Some(this) {
                            list.set_next(vm, c, following.clone());
                            break;
                        }
                        current = n;
                    }
                }
            }
            Value::Void
        }
        "SetLocation" => {
            // AActor::execSetLocation: ULevel::FarMoveActor.
            let moved = vm.far_move_actor(this, args[0].as_vector(), false, false, false);
            // Graphics: Original keeps the original's placement (the pawn falls).
            if moved {
                crate::improved::navigation::checkpoint_cine_to_floor(vm, this);
            }
            Value::Bool(moved)
        }
        "SetRotation" => {
            // AActor::execSetRotation: MoveActor without movement.
            let mut hit = crate::physics::CheckResult::new(1.0);
            Value::Bool(vm.move_actor(this, [0.0; 3], args[0].as_rotator(), &mut hit, false, false, false, false))
        }
        "SetPhysics" => {
            // AActor::execSetPhysics: setPhysics(NewPhysics, None, (0, 0, 1)).
            vm.set_physics(this, args[0].as_byte(), None, [0.0, 0.0, 1.0]);
            Value::Void
        }
        "SetDrawType" => {
            // AActor::execSetDrawType → AActor::SetDrawType: removed then
            // put back into the collision hash if bCollideActors (the shape depends on the DrawType),
            // bNetDirty.
            vm.set_prop(this, "DrawType", args[0].clone());
            if vm.propr(this, "bCollideActors").as_bool() {
                vm.refresh_collider(this);
            }
            vm.set_prop(this, "bNetDirty", Value::Bool(true));
            Value::Void
        }
        // AActor::execSetOwner: SetOwner(NewOwner).
        "SetOwner" => {
            vm.set_owner(this, args[0].as_object());
            Value::Void
        }
        "SetBase" => {
            // AActor::execSetBase: SetBase(NewBase, NewFloor = (0, 0, 1), 1).
            let floor = if supplied.get(1) == Some(&true) { args[1].as_vector() } else { [0.0, 0.0, 1.0] };
            vm.set_base_full(this, args[0].as_object(), floor, true);
            Value::Void
        }
        "SetCollision" => {
            // AActor::execSetCollision: omitted arguments keep the current value.
            let mut flags = [false; 3];
            for (k, p) in ["bCollideActors", "bBlockActors", "bBlockPlayers"].iter().enumerate() {
                flags[k] = if supplied.get(k) == Some(&true) { args[k].as_bool() } else { vm.propr(this, p).as_bool() };
            }
            vm.set_collision(this, flags[0], flags[1], flags[2]);
            Value::Void
        }
        "SetCollisionSize" => {
            // AActor::execSetCollisionSize.
            vm.set_collision_size(this, args[0].as_float(), args[1].as_float());
            Value::Bool(true)
        }
        "Init" | "ClientPlaySound" | "SetDefaultDisplayProperties" | "SetDisplayProperties" => Value::Void,
        // execIsInState: the current state or one of its parent states; without
        // a state, StateNode is the class and the chain is that of the classes.
        "IsInState" => Value::Bool(vm.state_chain_has(this, args.first().map(Value::name_ref).unwrap_or(""))),
        // execGetStateName: name of StateNode, the class outside any state.
        "GetStateName" => Value::Name(crate::value::NameStruct::from(vm.state_chain_names(this).into_iter().next().unwrap_or_default())),
        // execEnable / execDisable: outside the probe functions
        // (names 300 to 363), "Enable: '%s' is not a probe function" through Stack.Logf.
        "Enable" | "Disable" => {
            if crate::world::probe_bit(&s(0)).is_none() {
                // execEnable logs before swallowing the EndFunctionParms, execDisable after.
                let end = vm.call_ends.last().copied().unwrap_or(vm.cur_offset);
                let at = if name == "Enable" { end.saturating_sub(1) } else { end };
                vm.native_warn_at(at, &format!("Enable: '{}' is not a probe function", if s(0).is_empty() { "None".into() } else { s(0) }));
            }
            if let Some(bit) = crate::world::probe_bit(&s(0)) {
                let mask = vm.probe_disabled.entry(this).or_insert(0);
                if name == "Disable" {
                    *mask |= 1 << bit;
                } else {
                    *mask &= !(1 << bit);
                }
            }
            Value::Void
        }
        // UObject::execSaveConfig: SaveConfig(CPF_Config) of the object;
        // execStaticSaveConfig: that of its class's default values;
        // execResetConfig: UObject::ResetConfig of its class.
        "SaveConfig" => {
            vm.save_config(this, crate::config::CPF_CONFIG);
            Value::Void
        }
        "StaticSaveConfig" => {
            if let Some(class) = vm.objects[this].as_ref().map(|o| o.class) {
                let cdo = vm.classes[class].cdo;
                vm.save_config(cdo, crate::config::CPF_CONFIG);
            }
            Value::Void
        }
        "ResetConfig" => {
            if let Some(class) = vm.objects[this].as_ref().map(|o| o.class) {
                vm.reset_config(class);
            }
            Value::Void
        }
        // UObject::execGetPropertyText: public property of the class,
        // ExportText of the first element, the object also serving as the comparison value.
        "GetPropertyText" => {
            let class = vm.objects[this].as_ref().map(|o| o.class);
            let slot = class.and_then(|c| vm.classes[c].prop_index.get(&s(0).to_lowercase()).copied());
            match (class, slot) {
                (Some(c), Some(i)) if vm.classes[c].props[i].public => {
                    let def = vm.classes[c].props[i].clone();
                    let v = vm.objects[this].as_ref().map(|o| o.props[i].clone()).unwrap_or(Value::Void);
                    let first = match (&v, def.dim > 1) {
                        (Value::Array(a), true) => a.first().cloned().unwrap_or(Value::Void),
                        _ => v,
                    };
                    Value::Str(vm.export_text(&def.ty, def.enumeration.as_deref(), &first, Some(&first.clone()), 1 | crate::travel::EXPORT_ALL))
                }
                _ => Value::Str(String::new()),
            }
        }
        // execSetPropertyText: public and non-const property, ImportText of the
        // first element.
        "SetPropertyText" => {
            let class = vm.objects[this].as_ref().map(|o| o.class);
            let slot = class.and_then(|c| vm.classes[c].prop_index.get(&s(0).to_lowercase()).copied());
            if let (Some(c), Some(i)) = (class, slot) {
                let def = vm.classes[c].props[i].clone();
                if def.public && def.flags & 2 == 0 {
                    let chars: Vec<char> = s(1).chars().collect();
                    let mut v = vm.objects[this].as_ref().map(|o| o.props[i].clone()).unwrap_or(Value::Void);
                    if def.dim > 1 {
                        if let Value::Array(items) = &mut v {
                            if let Some(first) = items.first_mut() {
                                vm.import_text_named(&def.name, &def.ty, def.enumeration.as_deref(), &chars, 0, 1, first);
                            }
                        }
                    } else {
                        vm.import_text_named(&def.name, &def.ty, def.enumeration.as_deref(), &chars, 0, 1, &mut v);
                    }
                    if let Some(o) = vm.objects[this].as_mut() {
                        o.props[i] = v;
                    }
                }
            }
            Value::Void
        }
        _ => return Err(VmError::Native(format!("{name} not implemented"))),
    })
}

impl Vm {
    /// AActor::SetOwner: LostChild on the old owner, GainedChild on
    /// the new one, bNetDirty.
    pub(crate) fn set_owner(&mut self, actor: ObjId, owner: Option<ObjId>) {
        if let Some(old) = self.propr(actor, "Owner").as_object().filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) {
            self.event_pub(old, "LostChild", vec![Value::Object(Some(actor))]);
        }
        self.set_prop(actor, "Owner", Value::Object(owner));
        if let Some(new) = owner.filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) {
            self.event_pub(new, "GainedChild", vec![Value::Object(Some(actor))]);
        }
        self.set_prop(actor, "bNetDirty", Value::Bool(true));
    }
}

/// Result of an x87 operation at 24-bit precision (control word 0x007F): mantissa rounded
/// to 24 bits (to nearest, even), extended exponent (no float overflow).
/// Parent of class `name` (case-insensitive) among the classes of the loaded packages: those the VM does not keep (Class, StaticMesh, Mesh,
/// Font, SkeletalMesh…) are there too, with their parent line from Engine.u and Core.u.
/// Warning lines of a StaticLoadObject that fails with LOAD_NoWarn: path without "." (ResolveName
/// without a package: PackageResolveFailed written by SafeLoadError as "FailedLoad 'NULL'"), missing package
/// (PackageNotFound raised by GetPackageLinker, then the GObjPackageRemap attempt UnrealI → UnrealShare) or missing object
/// (ObjectNotFound); the catch writes FailedLoadObject. `has_package`: the package exists.
pub(crate) fn failure_static_load(vm: &mut Vm, requested: &str, path: &str, has_package: bool) -> Vec<String> {
    let mut lines_list = Vec::new();
    let failed_object = vm.localize("Errors", "FailedLoadObject", "Core", false);
    if !path.contains('.') {
        let resolve = vm.localize("Errors", "PackageResolveFailed", "Core", false);
        let failed = vm.localize("Errors", "FailedLoad", "Core", false);
        for (device, l1) in safe_load_error(LOAD_NO_WARN, &resolve, &failed, &["NULL", &resolve]) {
            if device == "GLog" {
                lines_list.push(format!("Warning: {l1}"));
            }
        }
        let l2 = crate::value::printf_s(&failed_object, &[requested, "None", path, &resolve]);
        lines_list.push(format!("Warning: {l2}"));
        return lines_list;
    }
    let (outer, short) = path.rsplit_once('.').map_or((String::new(), path.to_string()), |(o, n)| (o.to_string(), n.to_string()));
    let package = path.split('.').next().unwrap_or("").to_string();
    if has_package {
        let not_found = vm.localize("Errors", "ObjectNotFound", "Core", false);
        let err = crate::value::printf_s(&not_found, &[requested, &outer, &short]);
        let line = crate::value::printf_s(&failed_object, &[requested, &outer, &short, &err]);
        lines_list.push(format!("Warning: {line}"));
    } else {
        let failed = vm.localize("Errors", "FailedLoad", "Core", false);
        let no_package = vm.localize("Errors", "PackageNotFound", "Core", false);
        let err = crate::value::printf_s(&no_package, &[&package]);
        let l1 = crate::value::printf_s(&failed, &[&package, &err]);
        lines_list.push(format!("Warning: {l1}"));
        // GObjPackageRemap (UObject::StaticInit: UnrealI → UnrealShare): the catch redoes
        // StaticLoadObject with the replacement package (LOAD_NoRemap); neither package exists in XIII.
        let (outer, err) = if package.eq_ignore_ascii_case("UnrealI") && vm.library.as_mut().is_some_and(|l| l.get("UnrealShare").is_none()) {
            let err = crate::value::printf_s(&no_package, &["UnrealShare"]);
            let l = crate::value::printf_s(&failed, &["UnrealShare", &err]);
            let inner = crate::value::printf_s(&failed_object, &[requested, "UnrealShare", &short, &err]);
            lines_list.push(format!("Warning: {l}"));
            lines_list.push(format!("Warning: {inner}"));
            ("UnrealShare".to_string(), err)
        } else {
            (outer.clone(), err)
        };
        let l2 = crate::value::printf_s(&failed_object, &[requested, &outer, &short, &err]);
        lines_list.push(format!("Warning: {l2}"));
    }
    lines_list
}

/// StaticFindObject(UClass, ANY_PACKAGE, name) of a class that the VM does not have as a script class: a native class known to
/// the original (parent recorded at the real engine's startup, or class export of a loaded package: Sound, Texture, StaticMesh…),
/// with the spelling of its FName (that of the loader's messages).
pub(crate) fn class_native_known(vm: &mut Vm, name_value: &str) -> Option<String> {
    if name_value.is_empty() {
        return None;
    }
    if let Some((n, _)) = crate::classes_natives_table::PARENTS_NATIVES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name_value)) {
        return Some(n.to_string());
    }
    if name_value.eq_ignore_ascii_case("Object") {
        return Some("Object".to_string());
    }
    dyn_class_parent(vm, name_value).map(|_| name_value.to_string())
}

/// StaticLoadObject(class, NULL, path, NULL, LOAD_NoWarn, NULL) of a resource (object of a native class): the class of
/// the loaded export (Some: resource `Value::Asset(path)`), or None and the warning lines. Without a package
/// library, loading succeeds with the requested class.
pub(crate) fn load_resource(vm: &mut Vm, class_value: &str, path: &str) -> (Option<String>, Vec<String>) {
    if !path.contains('.') {
        return (None, failure_static_load(vm, class_value, path, false));
    }
    if vm.library.is_none() {
        return (Some(class_value.to_string()), Vec::new());
    }
    let package = path.split('.').next().unwrap_or("").to_string();
    let short = path.rsplit('.').next().unwrap_or(path).to_string();
    let classes: Option<Vec<String>> = vm.library.as_mut().and_then(|l| l.get(&package)).map(|lp| {
        let p = &lp.package;
        (0..p.exports.len()).filter(|&i| p.name(p.exports[i].object_name).eq_ignore_ascii_case(&short)).map(|i| p.export_class(i).to_string()).collect()
    });
    match classes {
        None => (None, failure_static_load(vm, class_value, path, false)),
        Some(cs) => match cs.into_iter().find(|c| dyn_class_is_a(vm, c, class_value)) {
            Some(c) => (Some(c), Vec::new()),
            None => (None, failure_static_load(vm, class_value, path, true)),
        },
    }
}

/// `class` is `ancestor` or derives from it (UObject::IsA on the class names, natives included).
pub(crate) fn class_is(vm: &mut Vm, class: &str, ancestor: &str) -> bool {
    dyn_class_is_a(vm, class, ancestor)
}

fn dyn_class_parent(vm: &mut Vm, name: &str) -> Option<String> {
    if vm.native_class_parents.is_none() {
        let mut map = std::collections::HashMap::new();
        for lp in &vm.packages {
            let p = &lp.package;
            for i in 0..p.exports.len() {
                if !p.export_class(i).eq_ignore_ascii_case("Class") {
                    continue;
                }
                if let Ok(c) = openxiii_script::objects::read_class(p, &lp.data, i) {
                    let sup = c.state.body.header.super_field;
                    if sup != 0 {
                        map.entry(p.name(p.exports[i].object_name).to_ascii_lowercase()).or_insert_with(|| p.object_name(sup.into()).to_string());
                    }
                }
            }
        }
        vm.native_class_parents = Some(map);
    }
    vm.native_class_parents
        .as_ref()
        .and_then(|m| m.get(&name.to_ascii_lowercase()).cloned())
        .or_else(|| crate::classes_natives_table::PARENTS_NATIVES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, m)| m.to_string()).filter(|m| !m.is_empty()))
}

/// `class` is `ancestor` or derives from it (UObject::IsA on the names).
fn dyn_class_is_a(vm: &mut Vm, class: &str, ancestor: &str) -> bool {
    let mut cur = class.to_string();
    for _ in 0..64 {
        if cur.eq_ignore_ascii_case(ancestor) {
            return true;
        }
        match dyn_class_parent(vm, &cur) {
            Some(p) => cur = p,
            None => return false,
        }
    }
    false
}

/// (the package exists, it has an export with this name whose class is `requested` or derives from it): FindExportIndex without regard to the group.
fn dyn_find_export(vm: &mut Vm, package: &str, short: &str, requested: &str) -> (bool, bool) {
    let classes: Option<Vec<String>> = vm.library.as_mut().and_then(|l| l.get(package)).map(|lp| {
        let p = &lp.package;
        (0..p.exports.len()).filter(|&i| p.name(p.exports[i].object_name).eq_ignore_ascii_case(short)).map(|i| p.export_class(i).to_string()).collect()
    });
    match classes {
        None => (false, false),
        Some(cs) => (true, cs.iter().any(|c| dyn_class_is_a(vm, c, requested))),
    }
}

pub(crate) fn x87_24(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let b = x.to_bits();
    let low_value = b & ((1u64 << 29) - 1);
    let mut h = b >> 29;
    if low_value > 1 << 28 || (low_value == 1 << 28 && h & 1 == 1) {
        h += 1;
    }
    f64::from_bits(h << 29)
}

/// "Indefinite" NaN that the x87 returns on an invalid operation (0xFFC00000).
const X87_NAN: f32 = f32::from_bits(0xFFC0_0000);

/// appRSqrt: 1 / sqrt(x) (FPU in single precision).
/// appAcos → msvcr70's _CIacos (0x7C007E94): atan2(sqrt((1 + x)(1 − x)), x)
/// through fpatan; the FPU's control keeps the 24-bit precision for the addition, the product and
/// the root. |x| > 1 gives NaN.
fn x87_acos(x: f32) -> f32 {
    if x.is_nan() {
        return x;
    }
    // Outside [-1, 1]: the x87's "indefinite" NaN (sign bit set).
    if x.abs() > 1.0 {
        return X87_NAN;
    }
    let s = ((1.0 + x) * (1.0 - x)).sqrt();
    (s as f64).datan2(x as f64) as f32
}

/// appExp: t = log2(e) · x rounded to 24 bits, n = frndint(t),
/// (f2xm1(t − n) + 1) rounded to 24 bits, then fscale by n.
fn x87_exp(x: f32) -> f32 {
    let t = (std::f64::consts::LOG2_E * x as f64) as f32;
    if t.is_nan() {
        return x;
    }
    // ±inf: t − frndint(t) = inf − inf, invalid operation → indefinite NaN.
    if x.is_infinite() {
        return X87_NAN;
    }
    if !t.is_finite() {
        return (x as f64).dexp() as f32;
    }
    let n = t.round_ties_even();
    let m = ((t - n) as f64).dexp2() as f32;
    (m as f64 * (n as f64).dexp2()) as f32
}

pub(crate) fn app_rsqrt(x: f32) -> f32 {
    1.0 / x.sqrt()
}

/// FVector::SafeNormal: zero if |V|² < 1e-8, otherwise V × appRSqrt(|V|²).
pub(crate) fn safe_normal(v: [f32; 3]) -> [f32; 3] {
    let sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if (sq as f64) < 1e-8 {
        return [0.0; 3];
    }
    let k = app_rsqrt(sq);
    [k * v[0], k * v[1], k * v[2]]
}

/// Optional integer parameter of a native: `default` if it is omitted (empty value).
fn opt_int(args: &[Value], supplied: &[bool], i: usize, default: i32) -> i32 {
    match args.get(i) {
        Some(v) if supplied.get(i) != Some(&false) && !matches!(v, Value::Void) => v.as_int(),
        _ => default,
    }
}

/// StaticFindObject for FindObject: a game class ("Package.Class", case-insensitive) or a
/// VM object ("Outer.Name") whose class derives from the requested class. A class is an object of class
/// Class: it passes the filter Class (`Value::Class(None)`), Object, Field, Struct and State. `None`: nothing like that in the
/// VM (the path can designate a package object, returned as an `Asset`).
fn find_object_vm(vm: &Vm, path: &str, filter_value: Option<&Value>) -> Option<Value> {
    let (package_value, _) = path.split_once('.')?;
    let pass_class = |f: Option<&Value>| match f {
        None | Some(Value::Object(None)) | Some(Value::Void) | Some(Value::Class(None)) => true,
        Some(v) => v.as_class().is_none_or(|k| ["Object", "Field", "Struct", "State"].iter().any(|n| vm.classes[k].name.eq_ignore_ascii_case(n))),
    };
    let package_of = |c: crate::value::ClassId| {
        // Mods build only: "Mod" for a mod's new class (mod_classes.rs).
        #[cfg(feature = "mods")]
        if let Some(p) = vm.mod_class_package(c) {
            return p;
        }
        vm.packages[vm.classes[c].package].name.as_str()
    };
    if let Some(c) = vm.class(path).filter(|&c| package_of(c).eq_ignore_ascii_case(package_value)) {
        return Some(if pass_class(filter_value) { Value::Class(Some(c)) } else { Value::Object(None) });
    }
    // ResolveName only goes down through packages (CreatePackage): an object inside another object has no path.
    let id = (0..vm.objects.len()).find(|&i| vm.objects[i].as_ref().is_some_and(|o| !o.outer.contains('.')) && vm.path_name(i).eq_ignore_ascii_case(path))?;
    let class_object = vm.objects[id].as_ref()?.class;
    let ok = match filter_value {
        None | Some(Value::Object(None)) | Some(Value::Void) => true,
        Some(Value::Class(None)) => false,
        Some(v) => v.as_class().is_none_or(|k| vm.is_child_of(class_object, k)),
    };
    Some(if ok { Value::Object(Some(id)) } else { Value::Object(None) })
}

