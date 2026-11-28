//! Definitions loaded from the packages: types, classes, structs, functions, states.

use crate::value::{ClassId, ObjId, Value};
use openxiii_script::bytecode::Expr;
/// Name tables of the definitions (properties, functions, states, labels): FxHash hashing, without
/// a random seed (no dependency on the order: RandomState's already changed at each
/// run), much cheaper than SipHash for these short keys read at every instruction.
pub type HashMap<K, V> = std::collections::HashMap<K, V, std::hash::BuildHasherDefault<FxHasher>>;

/// FxHasher, ported from the Rust compiler's `rustc-hash` (MIT OR Apache-2.0, see THIRD-PARTY.md):
/// word by word, rotation and multiplication.
#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(FX_SEED);
    }
}

impl std::hash::Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut b = bytes;
        while b.len() >= 8 {
            self.add(u64::from_le_bytes(b[..8].try_into().unwrap()));
            b = &b[8..];
        }
        if b.len() >= 4 {
            self.add(u32::from_le_bytes(b[..4].try_into().unwrap()) as u64);
            b = &b[4..];
        }
        for &x in b {
            self.add(x as u64);
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}
use std::rc::Rc;

pub type FuncId = usize;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Byte,
    Int,
    Bool,
    Float,
    Name,
    Str,
    Object(String),
    Class(String),
    Struct(String),
    Array(Box<Type>),
    Delegate,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct PropDef {
    pub name: String,
    pub ty: Type,
    /// Static array size (1 = plain value; otherwise stored as Value::Array).
    pub dim: usize,
    pub flags: u32,
    /// PropertyFlags of the element of a dynamic array (UArrayProperty::Inner): CPF 0x400000
    /// there says whether the array's objects are copied for each instance (sub_objects.rs); the
    /// array itself always carries 0x400000.
    pub inner_flags: u32,
    /// Enum of a byte property (or of the elements of a byte array): path
    /// "Package.[Class.]Enum", to import a name (UByteProperty::ImportText).
    pub enumeration: Option<String>,
    /// RF_Public of the export (absent from "private" properties): GetPropertyText and
    /// SetPropertyText only see those.
    pub public: bool,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    /// Parent struct ("struct Plane extends Vector"); its fields are at the head of `fields`.
    pub super_struct: Option<String>,
    pub fields: Vec<PropDef>,
    pub index: HashMap<String, usize>,
}

#[derive(Debug, Clone)]
pub struct ParamDef {
    pub local: usize,
    pub out: bool,
    pub optional: bool,
    pub coerce: bool,
}

/// Loaded code of a function or a state (bytecode + table position → instruction).
#[derive(Debug)]
pub struct Code {
    pub statements: Vec<Expr>,
    pub at: HashMap<u32, usize>,
    /// Package that the bytecode's object and name references come from.
    pub package: usize,
}

impl Code {
    pub fn new(statements: Vec<Expr>, package: usize) -> Rc<Code> {
        let at = statements.iter().enumerate().map(|(i, e)| (e.offset, i)).collect();
        Rc::new(Code { statements, at, package })
    }
}

#[derive(Debug)]
pub struct FunctionDef {
    pub name: String,
    pub owner: ClassId,
    pub state: Option<String>,
    pub package: usize,
    pub export: usize,
    pub flags: u32,
    pub native: u16,
    pub params: Vec<ParamDef>,
    pub locals: Vec<PropDef>,
    pub local_index: HashMap<String, usize>,
    pub ret: Option<Type>,
    pub code: Option<Rc<Code>>,
}

#[derive(Debug)]
pub struct StateDef {
    pub name: String,
    pub parent: Option<String>,
    /// Class (lowercase) that declares the parent state: UState::SuperField is a specific object,
    /// "state PlayerWalking extends PlayerWalking" refers to the state of the parent class.
    pub parent_class: Option<String>,
    pub functions: HashMap<String, FuncId>,
    pub code: Option<Rc<Code>>,
    /// Labels of the state code (lowercase name → memory position).
    pub labels: HashMap<String, u32>,
    pub ignores: Vec<String>,
    /// "Probe" events ignored in this state: bit at 0 = ignored (see `world::PROBES`).
    pub ignore_mask: u64,
    /// ProbeMask of the state (bit at 1 = event defined in the state; see `world::PROBES`).
    pub probe_mask: u64,
    /// Automatic state (`auto state`).
    pub auto: bool,
    /// STATE_Simulated (UState::StateFlags 4, `simulated state`): the only state code that runs on
    /// an actor without authority (AActor::ProcessState).
    pub simulated: bool,
}

#[derive(Debug)]
pub struct ClassDef {
    pub name: String,
    pub package: usize,
    pub export: usize,
    pub super_name: Option<String>,
    pub super_class: Option<ClassId>,
    /// Properties, inherited ones first.
    pub props: Vec<PropDef>,
    pub prop_index: HashMap<String, usize>,
    /// Functions declared by this class (outside states).
    pub functions: HashMap<String, FuncId>,
    pub states: HashMap<String, StateDef>,
    /// Default object of the class (defaultproperties values).
    pub cdo: ObjId,
    pub linked: bool,
    /// ClassFlags of the package (16 bits): CLASS_Config, CLASS_Localized, CLASS_PerObjectConfig…
    pub flags: u32,
    /// ClassConfigName: "System" (the game's ini), "User" (the player's ini) or an ini name.
    pub config_name: String,
    /// ProbeMask of the class (the class's UState, inherited bits included).
    pub probe_mask: u64,
}

/// Properties of an object, in its class's order. A "hot" property (bDeleteMe of
/// actors, read for every actor every frame) is stored in the structure itself, next to
/// the class, like a fixed-position field of the XIII object: it is the only copy of its value
/// (its slot in `vals` stays empty and is never shown). All accesses go through these methods.
///
/// `generation` changes at each access that can write (get_mut, IndexMut, iter_mut, with_slice,
/// push, truncate, resize), from scripts as from natives: a value computed with
/// the generation read stays correct as long as the generation has not changed. A new list starts from a
/// generation never given out (shared counter), a copy keeps the original's (same values).
#[derive(Clone)]
pub struct Props {
    vals: Vec<Value>,
    hot: Value,
    /// Slot of the hot property (usize::MAX: none).
    case: usize,
    generation: u64,
    /// Eligibility of the properties alone for the idle tick, per class binding.
    pub(crate) idle_tick: std::cell::Cell<Option<crate::tick::IdleHint>>,
}

/// Generation start of a new list: each one receives its range of 2^32 values (the high 32 bits).
fn generation_new() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 32);
    NEXT.fetch_add(1 << 32, std::sync::atomic::Ordering::Relaxed)
}

impl Props {
    /// `case`: slot of the class's hot property (or usize::MAX).
    pub fn new(mut vals: Vec<Value>, case: usize) -> Props {
        let hot = match vals.get_mut(case) {
            Some(v) => std::mem::replace(v, Value::Void),
            None => Value::Void,
        };
        let case = if case < vals.len() { case } else { usize::MAX };
        Props { vals, hot, case, generation: generation_new(), idle_tick: Default::default() }
    }

    /// Generation of the values: changes at each possible write (see `Props`).
    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[inline]
    fn writes(&mut self) {
        self.idle_tick.set(None);
        self.generation = self.generation.wrapping_add(1);
    }

    /// Write that does not change the generation, reserved for the tracking fields the engine sets
    /// every frame for every actor: LastRenderTime and LastRenderDist (FDynamicActor::Render),
    /// bTicked (AActor::Tick). No value kept per generation reads them (character signature, scenery entries,
    /// moved actors, collision shapes, emitter parameters), which would otherwise all be
    /// rebuilt every frame. Any new kept value that would read these two fields must
    /// compare them itself.
    pub fn set_render(&mut self, i: usize, v: Value) {
        if i == self.case {
            self.hot = v;
        } else if let Some(slot) = self.vals.get_mut(i) {
            *slot = v;
        }
    }

    /// Experiment (prefetching, tick.rs): address of the first slot (null if none).
    #[inline]
    pub fn first_case(&self) -> *const Value {
        if self.vals.is_empty() { std::ptr::null() } else { self.vals.as_ptr() }
    }

    /// Slot of the hot property (usize::MAX: none).
    pub fn case(&self) -> usize {
        self.case
    }

    pub fn len(&self) -> usize {
        self.vals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vals.is_empty()
    }

    #[inline]
    pub fn get(&self, i: usize) -> Option<&Value> {
        if i == self.case {
            return Some(&self.hot);
        }
        self.vals.get(i)
    }

    #[inline]
    pub fn get_mut(&mut self, i: usize) -> Option<&mut Value> {
        self.writes();
        if i == self.case {
            return Some(&mut self.hot);
        }
        self.vals.get_mut(i)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Value> + '_ {
        (0..self.vals.len()).map(move |i| &self[i])
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Value> + '_ {
        self.writes();
        let case = self.case;
        let hot = &mut self.hot;
        let mut hot = Some(hot);
        self.vals.iter_mut().enumerate().map(move |(i, v)| if i == case { hot.take().unwrap_or(v) } else { v })
    }

    /// The values in class order (copy).
    pub fn to_vec(&self) -> Vec<Value> {
        self.iter().cloned().collect()
    }

    /// The values in class order.
    pub fn into_vec(mut self) -> Vec<Value> {
        if let Some(v) = self.vals.get_mut(self.case) {
            *v = std::mem::replace(&mut self.hot, Value::Void);
        }
        self.vals
    }

    /// `f` on the values stored in class order, the hot property put back in its slot
    /// for the duration of the call.
    pub fn with_slice<R>(&mut self, f: impl FnOnce(&mut [Value]) -> R) -> R {
        self.writes();
        if let Some(v) = self.vals.get_mut(self.case) {
            *v = std::mem::replace(&mut self.hot, Value::Void);
        }
        let r = f(&mut self.vals);
        if let Some(v) = self.vals.get_mut(self.case) {
            self.hot = std::mem::replace(v, Value::Void);
        }
        r
    }

    pub fn push(&mut self, v: Value) {
        self.writes();
        self.vals.push(v);
    }

    pub fn truncate(&mut self, n: usize) {
        self.writes();
        if self.case >= n && self.case != usize::MAX {
            self.hot = Value::Void;
            self.case = usize::MAX;
        }
        self.vals.truncate(n);
    }

    pub fn resize(&mut self, n: usize, v: Value) {
        self.writes();
        if n < self.vals.len() {
            self.truncate(n);
        } else {
            self.vals.resize(n, v);
        }
    }
}

impl std::ops::Index<usize> for Props {
    type Output = Value;
    #[inline]
    fn index(&self, i: usize) -> &Value {
        if i == self.case {
            return &self.hot;
        }
        &self.vals[i]
    }
}

impl std::ops::IndexMut<usize> for Props {
    #[inline]
    fn index_mut(&mut self, i: usize) -> &mut Value {
        self.writes();
        if i == self.case {
            return &mut self.hot;
        }
        &mut self.vals[i]
    }
}

/// Without a hot property (the tests that build their objects): the list as is.
impl From<Vec<Value>> for Props {
    fn from(vals: Vec<Value>) -> Props {
        Props { vals, hot: Value::Void, case: usize::MAX, generation: generation_new(), idle_tick: Default::default() }
    }
}

impl FromIterator<Value> for Props {
    fn from_iter<I: IntoIterator<Item = Value>>(it: I) -> Props {
        Props::from(it.into_iter().collect::<Vec<_>>())
    }
}

impl Default for Props {
    fn default() -> Props {
        Props { vals: Vec::new(), hot: Value::Void, case: usize::MAX, generation: generation_new(), idle_tick: Default::default() }
    }
}

impl std::fmt::Debug for Props {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl PartialEq for Props {
    fn eq(&self, other: &Props) -> bool {
        self.len() == other.len() && self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

#[derive(Debug, Clone)]
pub struct Object {
    pub class: ClassId,
    pub props: Props,
    pub state: Option<String>,
    /// Class that declares the active state: StateFrame->StateNode is a specific UState (the
    /// automatic state of a parent class stays its own even if the class redefines it).
    pub state_class: Option<ClassId>,
    pub name: String,
    /// Path of the Outer (UObject::GetPathName of the Outer): the map's package for an actor,
    /// "Transient", or the object given to `new(Outer)`.
    pub outer: String,
}

#[cfg(test)]
mod generation_tests {
    use super::*;

    /// The generation changes at each write access, never on a read; a copy keeps
    /// its own; two new lists never have the same one.
    #[test]
    fn generation_properties() {
        let mut p = Props::new(vec![Value::Int(1), Value::Bool(false), Value::Int(3)], 1);
        let mut views_list = std::collections::HashSet::new();
        let g = p.generation();
        assert!(views_list.insert(g));
        let _ = (p.get(0), &p[2], p.iter().count(), p.to_vec());
        assert_eq!(p.generation(), g, "read");
        let writes_list: [fn(&mut Props); 7] = [
            |p| p[0] = Value::Int(5),
            |p| *p.get_mut(1).unwrap() = Value::Bool(true),
            |p| p.iter_mut().for_each(|_| ()),
            |p| p.with_slice(|_| ()),
            |p| p.push(Value::Int(9)),
            |p| p.truncate(3),
            |p| p.resize(4, Value::Void),
        ];
        for (k, e) in writes_list.iter().enumerate() {
            e(&mut p);
            assert!(views_list.insert(p.generation()), "write {k}");
        }
        // Engine tracking fields (Props::set_render): value written, generation unchanged,
        // hot slot included.
        let g = p.generation();
        p.set_render(0, Value::Int(7));
        p.set_render(1, Value::Bool(false));
        p.set_render(99, Value::Int(0));
        assert_eq!(p.generation(), g, "set_render");
        assert_eq!((&p[0], &p[1]), (&Value::Int(7), &Value::Bool(false)));
        let c = p.clone();
        assert_eq!(c.generation(), p.generation());
        for _ in 0..100 {
            assert!(views_list.insert(Props::from(vec![Value::Int(1)]).generation()));
            assert!(views_list.insert(Props::default().generation()));
        }
    }
}
