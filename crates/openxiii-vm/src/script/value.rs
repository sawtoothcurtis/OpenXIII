//! Values handled by the virtual machine.

use std::fmt;

pub type ObjId = usize;
pub type ClassId = usize;

/// Type name of a struct ("object.vector"…): a text that never changes, stored once
/// and for all (copying a struct no longer copies its name). Compared by its text, like the string
/// it replaces; displayed the same way.
/// A thin pointer (8 bytes, to the stored text): a Value fits in 32 bytes. Equality,
/// hashing and ordering are on the text (`&&str` delegates them to the text).
#[derive(Clone, Copy, Eq, Hash, PartialOrd, Ord)]
pub struct NameStruct(&'static &'static str);

impl NameStruct {
    pub const VECTOR: NameStruct = NameStruct(&"object.vector");
    pub const ROTATOR: NameStruct = NameStruct(&"object.rotator");
    /// Empty text (Delegate without a function).
    pub const EMPTY: NameStruct = NameStruct(&"");

    #[inline]
    pub fn as_str(&self) -> &'static str {
        self.0
    }

    /// Name made at runtime: stored in a table (each text only once).
    pub fn from_time(text_value: &str) -> NameStruct {
        type Table = std::collections::HashMap<&'static str, &'static &'static str>;
        static TABLE: std::sync::RwLock<Option<Table>> = std::sync::RwLock::new(None);
        if let Some(s) = TABLE.read().ok().and_then(|t| t.as_ref().and_then(|t| t.get(text_value).copied())) {
            return NameStruct(s);
        }
        let mut t = TABLE.write().unwrap_or_else(|e| e.into_inner());
        let t = t.get_or_insert_with(Default::default);
        if let Some(&s) = t.get(text_value) {
            return NameStruct(s);
        }
        let s: &'static str = Box::leak(text_value.to_string().into_boxed_str());
        let r: &'static &'static str = Box::leak(Box::new(s));
        t.insert(s, r);
        NameStruct(r)
    }

    /// Literal text (`"object.color".into()`): kept by its address, without going through the table.
    fn literal(s: &'static str) -> NameStruct {
        thread_local! {
            static SEEN: std::cell::RefCell<std::collections::HashMap<(usize, usize), NameStruct, std::hash::BuildHasherDefault<crate::types::FxHasher>>> = Default::default();
        }
        let key_tuple = (s.as_ptr() as usize, s.len());
        if let Some(n) = SEEN.with(|v| v.borrow().get(&key_tuple).copied()) {
            return n;
        }
        let n = NameStruct::from_time(s);
        SEEN.with(|v| v.borrow_mut().insert(key_tuple, n));
        n
    }
}

impl std::ops::Deref for NameStruct {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.0
    }
}

impl PartialEq for NameStruct {
    #[inline]
    fn eq(&self, o: &NameStruct) -> bool {
        // Same pointer: same text (common case, stored names); otherwise the text.
        std::ptr::eq(self.0, o.0) || *self.0 == *o.0
    }
}

impl PartialEq<str> for NameStruct {
    fn eq(&self, o: &str) -> bool {
        *self.0 == o
    }
}

impl PartialEq<&str> for NameStruct {
    fn eq(&self, o: &&str) -> bool {
        *self.0 == *o
    }
}

impl PartialEq<String> for NameStruct {
    fn eq(&self, o: &String) -> bool {
        *self.0 == o.as_str()
    }
}

impl PartialEq<NameStruct> for &str {
    fn eq(&self, o: &NameStruct) -> bool {
        *self == *o.0
    }
}

impl PartialEq<NameStruct> for String {
    fn eq(&self, o: &NameStruct) -> bool {
        self.as_str() == *o.0
    }
}

impl From<&'static str> for NameStruct {
    fn from(s: &'static str) -> NameStruct {
        NameStruct::literal(s)
    }
}

impl From<String> for NameStruct {
    fn from(s: String) -> NameStruct {
        NameStruct::from_time(&s)
    }
}

impl From<&String> for NameStruct {
    fn from(s: &String) -> NameStruct {
        NameStruct::from_time(s)
    }
}

impl From<NameStruct> for String {
    fn from(s: NameStruct) -> String {
        s.0.to_string()
    }
}

impl AsRef<str> for NameStruct {
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl std::borrow::Borrow<str> for NameStruct {
    fn borrow(&self) -> &str {
        self.0
    }
}

impl fmt::Debug for NameStruct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0, f)
    }
}

impl fmt::Display for NameStruct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0, f)
    }
}

/// Fields of a struct (vector, rotator…): a boxed slice (16 bytes, a single
/// indirection, like the previous Vec), so that a Value fits in 32 bytes. Same value, same
/// order, same comparisons and same display as a `Vec<Value>`.
#[derive(Clone, PartialEq, Default)]
pub struct Fields(Box<[Value]>);

impl Fields {
    #[inline]
    pub fn new(v: Vec<Value>) -> Fields {
        Fields(v.into_boxed_slice())
    }
    pub fn into_vec(self) -> Vec<Value> {
        self.0.into_vec()
    }
}

impl std::ops::Deref for Fields {
    type Target = [Value];
    #[inline]
    fn deref(&self) -> &[Value] {
        &self.0
    }
}

impl std::ops::DerefMut for Fields {
    #[inline]
    fn deref_mut(&mut self) -> &mut [Value] {
        &mut self.0
    }
}

impl From<Vec<Value>> for Fields {
    #[inline]
    fn from(v: Vec<Value>) -> Fields {
        Fields::new(v)
    }
}

impl From<Fields> for Vec<Value> {
    fn from(c: Fields) -> Vec<Value> {
        c.into_vec()
    }
}

impl FromIterator<Value> for Fields {
    fn from_iter<I: IntoIterator<Item = Value>>(it: I) -> Fields {
        Fields(it.into_iter().collect())
    }
}

impl IntoIterator for Fields {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<'a> IntoIterator for &'a Fields {
    type Item = &'a Value;
    type IntoIter = std::slice::Iter<'a, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a mut Fields {
    type Item = &'a mut Value;
    type IntoIter = std::slice::IterMut<'a, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

impl fmt::Debug for Fields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}
/// `Vector` / `Rotator`: inline forms (unboxed) of `Struct("object.vector", [Float ×3])` and
/// `Struct("object.rotator", [Int ×3])`. The two forms are the SAME value: equality, `{:?}`, text
/// and fields (`struct_view`, `struct_mut`) treat them alike; only the memory layout changes.
#[derive(Clone)]
pub enum Value {
    /// No value (procedure, optional parameter omitted).
    Void,
    Byte(u8),
    Int(i32),
    Bool(bool),
    Float(f32),
    Name(NameStruct),
    Str(String),
    Object(Option<ObjId>),
    /// Data object outside the VM (texture, sound, mesh…), designated by its path.
    Asset(String),
    Class(Option<ClassId>),
    /// Struct: type name and fields in declaration order.
    Struct(NameStruct, Fields),
    Array(Vec<Value>),
    /// Delegate: object and function name.
    Delegate(Option<ObjId>, NameStruct),
    /// `Struct("object.vector", [Float(x), Float(y), Float(z)])` inline.
    Vector([f32; 3]),
    /// `Struct("object.rotator", [Int(p), Int(y), Int(r)])` inline.
    Rotator([i32; 3]),
}

/// Fields of a struct seen for reading: the slice of a `Struct`, or the three values of an
/// inline `Vector` / `Rotator` (made on the fly, without allocation).
pub enum ViewFields<'a> {
    Slice(&'a [Value]),
    Three([Value; 3]),
}

impl std::ops::Deref for ViewFields<'_> {
    type Target = [Value];
    #[inline]
    fn deref(&self) -> &[Value] {
        match self {
            ViewFields::Slice(t) => t,
            ViewFields::Three(t) => t,
        }
    }
}

impl fmt::Debug for Value {
    /// Same text as `#[derive(Debug)]`; `Vector` / `Rotator` display as the `Struct` they are.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Void => f.write_str("Void"),
            Value::Byte(v) => f.debug_tuple("Byte").field(v).finish(),
            Value::Int(v) => f.debug_tuple("Int").field(v).finish(),
            Value::Bool(v) => f.debug_tuple("Bool").field(v).finish(),
            Value::Float(v) => f.debug_tuple("Float").field(v).finish(),
            Value::Name(v) => f.debug_tuple("Name").field(v).finish(),
            Value::Str(v) => f.debug_tuple("Str").field(v).finish(),
            Value::Object(v) => f.debug_tuple("Object").field(v).finish(),
            Value::Asset(v) => f.debug_tuple("Asset").field(v).finish(),
            Value::Class(v) => f.debug_tuple("Class").field(v).finish(),
            Value::Struct(n, c) => f.debug_tuple("Struct").field(n).field(c).finish(),
            Value::Array(v) => f.debug_tuple("Array").field(v).finish(),
            Value::Delegate(o, n) => f.debug_tuple("Delegate").field(o).field(n).finish(),
            Value::Vector(_) | Value::Rotator(_) => {
                let (n, c) = self.struct_view().expect("inline form");
                f.debug_tuple("Struct").field(&n).field(&&*c).finish()
            }
        }
    }
}

impl PartialEq for Value {
    /// Same equality as `#[derive(PartialEq)]`; an inline form equals the `Struct` it is.
    fn eq(&self, o: &Value) -> bool {
        use Value::*;
        match (self, o) {
            (Void, Void) => true,
            (Byte(a), Byte(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Bool(a), Bool(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Name(a), Name(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Object(a), Object(b)) => a == b,
            (Asset(a), Asset(b)) => a == b,
            (Class(a), Class(b)) => a == b,
            (Struct(n, a), Struct(m, b)) => n == m && a == b,
            (Array(a), Array(b)) => a == b,
            (Delegate(a, n), Delegate(b, m)) => a == b && n == m,
            // Float == Float component by component (NaN not equal to itself, -0 equal to 0), as before.
            (Vector(a), Vector(b)) => a[0] == b[0] && a[1] == b[1] && a[2] == b[2],
            (Rotator(a), Rotator(b)) => a == b,
            (Vector(_) | Rotator(_), Struct(..) | Vector(_) | Rotator(_)) | (Struct(..), Vector(_) | Rotator(_)) => {
                let (Some((n, a)), Some((m, b))) = (self.struct_view(), o.struct_view()) else { return false };
                n == m && *a == *b
            }
            _ => false,
        }
    }
}

impl Value {
    /// Name and fields of a struct, whatever its form (`Struct`, `Vector`, `Rotator`).
    #[inline]
    pub fn struct_view(&self) -> Option<(NameStruct, ViewFields<'_>)> {
        match self {
            Value::Struct(n, c) => Some((*n, ViewFields::Slice(c))),
            Value::Vector(v) => Some((NameStruct::VECTOR, ViewFields::Three(v.map(Value::Float)))),
            Value::Rotator(r) => Some((NameStruct::ROTATOR, ViewFields::Three(r.map(Value::Int)))),
            _ => None,
        }
    }

    /// Field `i` of a struct (copy), whatever its form; None outside a struct or out of range.
    #[inline]
    pub fn field_value(&self, i: usize) -> Option<Value> {
        match self {
            Value::Struct(_, c) => c.get(i).cloned(),
            Value::Vector(v) => v.get(i).map(|&x| Value::Float(x)),
            Value::Rotator(r) => r.get(i).map(|&x| Value::Int(x)),
            _ => None,
        }
    }

    /// Is there a struct (any form)?
    #[inline]
    pub fn is_struct(&self) -> bool {
        matches!(self, Value::Struct(..) | Value::Vector(_) | Value::Rotator(_))
    }

    /// `Struct` form in place (an inline form is unfolded into it), to write field by field.
    pub fn struct_mut(&mut self) -> Option<(&mut NameStruct, &mut Fields)> {
        if let Some(s) = self.unfold() {
            *self = s;
        }
        match self {
            Value::Struct(n, c) => Some((n, c)),
            _ => None,
        }
    }

    /// `Struct` form (owned name and fields), whatever the form.
    pub fn into_struct(self) -> Option<(NameStruct, Fields)> {
        match self {
            Value::Struct(n, c) => Some((n, c)),
            v @ (Value::Vector(_) | Value::Rotator(_)) => match v.unfold() {
                Some(Value::Struct(n, c)) => Some((n, c)),
                _ => None,
            },
            _ => None,
        }
    }

    /// The `Struct` that an inline form is (None for any other value).
    pub fn unfold(&self) -> Option<Value> {
        match self {
            Value::Vector(v) => Some(Value::Struct(NameStruct::VECTOR, v.map(Value::Float).to_vec().into())),
            Value::Rotator(r) => Some(Value::Struct(NameStruct::ROTATOR, r.map(Value::Int).to_vec().into())),
            _ => None,
        }
    }

    /// Inline form of a vector `Struct` (3 Float) or rotator `Struct` (3 Int); any other value returned as is.
    pub fn fold(self) -> Value {
        match &self {
            Value::Struct(n, c) if c.len() == 3 && *n == NameStruct::VECTOR => match **c {
                [Value::Float(x), Value::Float(y), Value::Float(z)] => Value::Vector([x, y, z]),
                _ => self,
            },
            Value::Struct(n, c) if c.len() == 3 && *n == NameStruct::ROTATOR => match **c {
                [Value::Int(p), Value::Int(y), Value::Int(r)] => Value::Rotator([p, y, r]),
                _ => self,
            },
            _ => self,
        }
    }

    /// `Struct("object.vector", [Float(x), Float(y), Float(z)])`, inline.
    #[inline]
    pub fn vector(x: f32, y: f32, z: f32) -> Value {
        Value::Vector([x, y, z])
    }

    /// `Struct("object.rotator", [Int(p), Int(y), Int(r)])`, inline.
    #[inline]
    pub fn rotator(p: i32, y: i32, r: i32) -> Value {
        Value::Rotator([p, y, r])
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Byte(b) => *b != 0,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Object(o) => o.is_some(),
            Value::Class(c) => c.is_some(),
            Value::Asset(_) => true,
            Value::Name(n) => !n.is_empty() && !n.eq_ignore_ascii_case("None"),
            Value::Str(s) => !s.is_empty(),
            _ => false,
        }
    }

    pub fn as_int(&self) -> i32 {
        match self {
            Value::Int(i) => *i,
            Value::Byte(b) => *b as i32,
            Value::Bool(b) => *b as i32,
            Value::Float(f) => *f as i32,
            Value::Str(s) => s.trim().parse::<f64>().map(|v| v as i32).unwrap_or(0),
            _ => 0,
        }
    }

    pub fn as_float(&self) -> f32 {
        match self {
            Value::Float(f) => *f,
            Value::Int(i) => *i as f32,
            Value::Byte(b) => *b as f32,
            Value::Bool(b) => *b as i32 as f32,
            Value::Str(s) => s.trim().parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    pub fn as_byte(&self) -> u8 {
        match self {
            Value::Byte(b) => *b,
            Value::Float(f) => *f as i32 as u8,
            other => other.as_int() as u8,
        }
    }

    pub fn as_object(&self) -> Option<ObjId> {
        match self {
            Value::Object(o) => *o,
            _ => None,
        }
    }

    pub fn as_class(&self) -> Option<ClassId> {
        match self {
            Value::Class(c) => *c,
            _ => None,
        }
    }

    pub fn as_name(&self) -> String {
        match self {
            Value::Name(n) => String::from(n.clone()),
            Value::Str(s) => s.clone(),
            _ => String::new(),
        }
    }

    /// as_name without copying.
    pub fn name_ref(&self) -> &str {
        match self {
            Value::Name(n) => n.as_str(),
            Value::Str(s) => s,
            _ => "",
        }
    }

    /// Components of a vector (or of a rotator converted to floats).
    pub fn as_vector(&self) -> [f32; 3] {
        match self {
            Value::Vector(v) => *v,
            // as_float of an Int: i32 → f32.
            Value::Rotator(r) => r.map(|i| i as f32),
            Value::Struct(_, f) if f.len() >= 3 => [f[0].as_float(), f[1].as_float(), f[2].as_float()],
            _ => [0.0; 3],
        }
    }

    pub fn as_rotator(&self) -> [i32; 3] {
        match self {
            Value::Rotator(r) => *r,
            // as_int of a Float: f32 → i32 (Rust's saturating truncation).
            Value::Vector(v) => v.map(|x| x as i32),
            Value::Struct(_, f) if f.len() >= 3 => [f[0].as_int(), f[1].as_int(), f[2].as_int()],
            _ => [0; 3],
        }
    }
}

/// msvcr70's _ftol: truncation to a 64-bit integer; out of range or NaN,
/// the undefined integer 0x8000000000000000.
pub fn ftol(f: f32) -> i64 {
    if f.is_nan() || f.abs() >= 9.223_372e18 {
        i64::MIN
    } else {
        f as i64
    }
}

/// appStrfind: first occurrence of `find` ignoring case (letters a to z), but
/// only at the start of a word: not right after a letter or a digit.
pub fn app_strfind(s: &str, find: &str) -> Option<usize> {
    let (s, f) = (s.as_bytes(), find.as_bytes());
    let first = f.first()?.to_ascii_uppercase();
    let mut in_word = false;
    for i in 0..s.len() {
        let c = s[i].to_ascii_uppercase();
        if !in_word && c == first && s.len() - i >= f.len() && s[i + 1..i + f.len()].eq_ignore_ascii_case(&f[1..]) {
            return Some(i);
        }
        in_word = c.is_ascii_uppercase() || c.is_ascii_digit();
    }
    None
}

/// Parse(Stream, Match, FString&): the value after `key` found
/// by appStrfind; in quotes up to the next one, otherwise up to a space, CR, LF,
/// tab or comma (at most 4095 characters).
pub fn parse_fstring(stream: &str, key: &str) -> Option<String> {
    let at = app_strfind(stream, key)?;
    let rest = &stream[at + key.len()..];
    let v = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or(""),
        None => rest.split([' ', '\r', '\n', '\t', ',']).next().unwrap_or(""),
    };
    Some(v.chars().take(0xFFF).collect())
}

/// Parse(Stream, Match, int&): appStrfind then appAtoi right after the key, without a token
/// ("12abc" gives 12, "abc" gives 0, and the key is "found").
pub fn parse_int(stream: &str, key: &str) -> Option<i32> {
    let at = app_strfind(stream, key)?;
    let rest: Vec<char> = stream[at + key.len()..].chars().collect();
    Some(crate::config::atoi(&rest, 0))
}

/// appSprintf of a format whose arguments are all "%s" (localized messages).
pub fn printf_s(format: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = args.iter();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' && chars.peek() == Some(&'s') {
            chars.next();
            out.push_str(rest.next().copied().unwrap_or(""));
        } else {
            out.push(c);
        }
    }
    out
}

/// "%f" of _vsnprintf (msvcr70): moved to openxiii-assets (log of ATerrainInfo::PostLoad).
pub use openxiii_assets::printf::{printf_f, printf_f_prec};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_as_core() {
        // appStrfind: start of word only, ignoring case.
        assert_eq!(parse_fstring("SubName=x Name=y", "NAME=").as_deref(), Some("y"));
        assert_eq!(parse_fstring("Class=XIII.Fists Name=Fists0", "CLASS=").as_deref(), Some("XIII.Fists"));
        assert_eq!(parse_fstring("A=\"b c\",d", "a=").as_deref(), Some("b c"));
        assert_eq!(parse_fstring("A=1,2", "A=").as_deref(), Some("1"));
        assert_eq!(parse_fstring("XA=1", "A="), None);
    }

    #[test]
    fn printf_f_like_msvcrt() {
        assert_eq!(printf_f(0.0), "0.000000");
        assert_eq!(printf_f(1.5), "1.500000");
        assert_eq!(printf_f(-2.25), "-2.250000");
        // 0.0078125 (2^-7): equality at the seventh digit, rounded up like _cftof.
        assert_eq!(printf_f(0.0078125), "0.007813");
        assert_eq!(printf_f(123456.789), "123456.789063");
        assert_eq!(printf_f(9.9999999), "10.000000");
        assert_eq!(printf_f(1e-8), "0.000000");
        assert_eq!(ftol(-3.9), -3);
        assert_eq!(ftol(f32::NAN) as i32, 0);
        assert_eq!(ftol(300.7) as u8, 44);
    }
}

/// Float to text conversion as in UnrealScript (two decimals).
pub fn float_text(f: f32) -> String {
    format!("{f:.2}")
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Void => Ok(()),
            Value::Byte(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Bool(b) => f.write_str(if *b { "True" } else { "False" }),
            Value::Float(x) => f.write_str(&float_text(*x)),
            Value::Name(n) => f.write_str(if n.is_empty() { "None" } else { n }),
            Value::Str(s) => f.write_str(s),
            Value::Object(None) | Value::Class(None) => f.write_str("None"),
            // Logs and debugging only: scripts convert objects with their path name (ObjectToString).
            Value::Object(Some(o)) => write!(f, "Object{o}"),
            Value::Class(Some(c)) => write!(f, "Class{c}"),
            Value::Asset(p) => f.write_str(p),
            Value::Struct(name, fields) if name.ends_with("vector") || name.ends_with("rotator") => {
                let parts: Vec<String> = fields.iter().map(|v| v.to_string()).collect();
                f.write_str(&parts.join(","))
            }
            Value::Struct(name, fields) => write!(f, "{name}({})", fields.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")),
            Value::Array(items) => write!(f, "[{}]", items.len()),
            Value::Delegate(_, n) => f.write_str(n),
            Value::Vector(_) | Value::Rotator(_) => {
                let (_, c) = self.struct_view().expect("inline form");
                let parts: Vec<String> = c.iter().map(|v| v.to_string()).collect();
                f.write_str(&parts.join(","))
            }
        }
    }
}

// A Value fits in 32 bytes (cache: the properties of all the actors).
const _: () = assert!(std::mem::size_of::<Value>() == 32);
