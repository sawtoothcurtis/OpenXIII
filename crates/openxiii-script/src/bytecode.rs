//! UnrealScript bytecode (Unreal Engine 2, package version 100).
//!
//! On disk, object references and names are compact indices; in
//! memory they take 4 bytes each. `ScriptSize` is the in-memory size: the
//! reader recomputes it token by token and stops exactly at that size, which
//! validates the decoding. Jumps (`Jump`, `JumpIfNot`…) target memory offsets.

use openxiii_package::Reader;
use std::fmt;

#[derive(Debug)]
pub enum ScriptError {
    Package(openxiii_package::Error),
    /// Unknown token (value, position in the file).
    UnknownToken(u8, usize),
    /// Expressions nested deeper than `MAX_DEPTH` at this file position (damaged script).
    TooDeep(usize),
    /// Recomputed memory size differs from ScriptSize.
    SizeMismatch {
        expected: u32,
        got: u32,
    },
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptError::Package(e) => write!(f, "{e}"),
            ScriptError::UnknownToken(t, at) => write!(f, "unknown token 0x{t:02X} at 0x{at:X}"),
            ScriptError::TooDeep(at) => write!(f, "expressions nested too deeply at 0x{at:X}"),
            ScriptError::SizeMismatch { expected, got } => {
                write!(f, "memory size {got} instead of {expected}")
            }
        }
    }
}

impl std::error::Error for ScriptError {}

impl From<openxiii_package::Error> for ScriptError {
    fn from(e: openxiii_package::Error) -> Self {
        ScriptError::Package(e)
    }
}

pub type Result<T> = std::result::Result<T, ScriptError>;

/// An expression (token and its operands), with its memory offset.
#[derive(Debug, Clone)]
pub struct Expr {
    pub offset: u32,
    /// Memory offset following the expression (Stack.Code after it is evaluated).
    pub end: u32,
    pub kind: Kind,
}

#[derive(Debug, Clone)]
pub enum Kind {
    LocalVariable(i32),
    InstanceVariable(i32),
    DefaultVariable(i32),
    Return(Box<Expr>),
    Switch(u8, Box<Expr>),
    Jump(u16),
    JumpIfNot(u16, Box<Expr>),
    Stop,
    Assert(u16, Box<Expr>),
    /// Offset of the next case (0xFFFF = default) and the case value.
    Case(u16, Option<Box<Expr>>),
    Nothing,
    LabelTable(Vec<(i32, u32)>),
    GotoLabel(Box<Expr>),
    EatString(Box<Expr>),
    Let(Box<Expr>, Box<Expr>),
    DynArrayElement(Box<Expr>, Box<Expr>),
    New(Box<Expr>, Box<Expr>, Box<Expr>, Box<Expr>),
    ClassContext(Box<Expr>, u16, u8, Box<Expr>),
    MetaCast(i32, Box<Expr>),
    LetBool(Box<Expr>, Box<Expr>),
    EndFunctionParms,
    SelfObject,
    Skip(u16, Box<Expr>),
    Context(Box<Expr>, u16, u8, Box<Expr>),
    ArrayElement(Box<Expr>, Box<Expr>),
    VirtualFunction(i32, Vec<Expr>),
    FinalFunction(i32, Vec<Expr>),
    GlobalFunction(i32, Vec<Expr>),
    NativeFunction(u16, Vec<Expr>),
    IntConst(i32),
    FloatConst(f32),
    StringConst(String),
    ObjectConst(i32),
    NameConst(i32),
    RotationConst([i32; 3]),
    VectorConst([f32; 3]),
    ByteConst(u8),
    IntZero,
    IntOne,
    True,
    False,
    NativeParm(i32),
    NoObject,
    IntConstByte(u8),
    BoolVariable(Box<Expr>),
    DynamicCast(i32, Box<Expr>),
    Iterator(Box<Expr>, u16),
    IteratorPop,
    IteratorNext,
    StructCmpEq(i32, Box<Expr>, Box<Expr>),
    StructCmpNe(i32, Box<Expr>, Box<Expr>),
    UnicodeStringConst(String),
    StructMember(i32, Box<Expr>),
    DynArrayLength(Box<Expr>),
    DynArrayInsert(Box<Expr>, Box<Expr>, Box<Expr>),
    DynArrayRemove(Box<Expr>, Box<Expr>, Box<Expr>),
    /// Delegate call: property, function name, parameters.
    DelegateFunction(i32, i32, Vec<Expr>),
    DelegateProperty(i32),
    LetDelegate(Box<Expr>, Box<Expr>),
    /// Type conversion (conversion code, e.g. 0x3F = IntToFloat) and the converted expression.
    Conversion(u8, Box<Expr>),
}

pub struct ScriptReader<'a, 'b> {
    r: &'b mut Reader<'a>,
    /// Current memory offset.
    pub mem: u32,
    /// Indices of the package's "None" names (end of the label table); XIII has several.
    none: &'b [i32],
    /// Nesting of the expression being read.
    depth: u32,
}

/// Deepest expression nesting read; far above what a compiled script contains, it only stops a
/// damaged one before the reader (and later walks of the tree) run out of stack.
pub const MAX_DEPTH: u32 = 512;

impl<'a, 'b> ScriptReader<'a, 'b> {
    pub fn new(r: &'b mut Reader<'a>, none: &'b [i32]) -> Self {
        ScriptReader { r, mem: 0, none, depth: 0 }
    }

    fn u8(&mut self) -> Result<u8> {
        self.mem += 1;
        Ok(self.r.u8()?)
    }
    fn u16(&mut self) -> Result<u16> {
        self.mem += 2;
        Ok(self.r.u16()?)
    }
    fn i32(&mut self) -> Result<i32> {
        self.mem += 4;
        Ok(self.r.i32()?)
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.i32()? as u32))
    }
    fn object(&mut self) -> Result<i32> {
        self.mem += 4;
        Ok(self.r.compact()?)
    }
    fn name(&mut self) -> Result<i32> {
        self.mem += 4;
        Ok(self.r.compact()?)
    }
    fn expr(&mut self) -> Result<Box<Expr>> {
        Ok(Box::new(self.read()?))
    }

    /// Reads a complete expression.
    pub fn read(&mut self) -> Result<Expr> {
        if self.depth >= MAX_DEPTH {
            return Err(ScriptError::TooDeep(self.r.pos));
        }
        self.depth += 1;
        let e = self.read_inner();
        self.depth -= 1;
        e
    }

    /// Call parameters up to `EndFunctionParms` (consumed, not returned).
    fn params(&mut self) -> Result<Vec<Expr>> {
        let mut params = Vec::new();
        loop {
            let e = self.read()?;
            if matches!(e.kind, Kind::EndFunctionParms) {
                return Ok(params);
            }
            params.push(e);
        }
    }

    fn read_inner(&mut self) -> Result<Expr> {
        let offset = self.mem;
        let at = self.r.pos;
        let token = self.u8()?;
        let kind = match token {
            0x00 => Kind::LocalVariable(self.object()?),
            0x01 => Kind::InstanceVariable(self.object()?),
            0x02 => Kind::DefaultVariable(self.object()?),
            0x04 => Kind::Return(self.expr()?),
            0x05 => Kind::Switch(self.u8()?, self.expr()?),
            0x06 => Kind::Jump(self.u16()?),
            0x07 => Kind::JumpIfNot(self.u16()?, self.expr()?),
            0x08 => Kind::Stop,
            0x09 => Kind::Assert(self.u16()?, self.expr()?),
            0x0A => {
                let next = self.u16()?;
                let value = if next != 0xFFFF { Some(self.expr()?) } else { None };
                Kind::Case(next, value)
            }
            0x0B => Kind::Nothing,
            0x0C => {
                let mut labels = Vec::new();
                loop {
                    // On disk: compact name + u32; in memory: name (4) + offset (4).
                    let name = self.name()?;
                    let offset = self.i32()? as u32;
                    labels.push((name, offset));
                    if self.none.contains(&name) {
                        break;
                    }
                }
                Kind::LabelTable(labels)
            }
            0x0D => Kind::GotoLabel(self.expr()?),
            0x0E => Kind::EatString(self.expr()?),
            0x0F => Kind::Let(self.expr()?, self.expr()?),
            0x10 => Kind::DynArrayElement(self.expr()?, self.expr()?),
            0x11 => Kind::New(self.expr()?, self.expr()?, self.expr()?, self.expr()?),
            0x12 => Kind::ClassContext(self.expr()?, self.u16()?, self.u8()?, self.expr()?),
            0x13 => Kind::MetaCast(self.object()?, self.expr()?),
            0x14 => Kind::LetBool(self.expr()?, self.expr()?),
            0x16 => Kind::EndFunctionParms,
            0x17 => Kind::SelfObject,
            0x18 => Kind::Skip(self.u16()?, self.expr()?),
            0x19 => Kind::Context(self.expr()?, self.u16()?, self.u8()?, self.expr()?),
            0x1A => Kind::ArrayElement(self.expr()?, self.expr()?),
            0x1B => Kind::VirtualFunction(self.name()?, self.params()?),
            0x1C => Kind::FinalFunction(self.object()?, self.params()?),
            0x1D => Kind::IntConst(self.i32()?),
            0x1E => Kind::FloatConst(self.f32()?),
            0x1F => {
                let mut s = String::new();
                loop {
                    let c = self.u8()?;
                    if c == 0 {
                        break;
                    }
                    s.push(c as char);
                }
                Kind::StringConst(s)
            }
            0x20 => Kind::ObjectConst(self.object()?),
            0x21 => Kind::NameConst(self.name()?),
            0x22 => Kind::RotationConst([self.i32()?, self.i32()?, self.i32()?]),
            0x23 => Kind::VectorConst([self.f32()?, self.f32()?, self.f32()?]),
            0x24 => Kind::ByteConst(self.u8()?),
            0x25 => Kind::IntZero,
            0x26 => Kind::IntOne,
            0x27 => Kind::True,
            0x28 => Kind::False,
            0x29 => Kind::NativeParm(self.object()?),
            0x2A => Kind::NoObject,
            0x2C => Kind::IntConstByte(self.u8()?),
            0x2D => Kind::BoolVariable(self.expr()?),
            0x2E => Kind::DynamicCast(self.object()?, self.expr()?),
            0x2F => Kind::Iterator(self.expr()?, self.u16()?),
            0x30 => Kind::IteratorPop,
            0x31 => Kind::IteratorNext,
            0x32 => Kind::StructCmpEq(self.object()?, self.expr()?, self.expr()?),
            0x33 => Kind::StructCmpNe(self.object()?, self.expr()?, self.expr()?),
            0x34 => {
                let mut units = Vec::new();
                loop {
                    let c = self.u16()?;
                    if c == 0 {
                        break;
                    }
                    units.push(c);
                }
                // appFromUnicode (Core): a character above 0xFF becomes 0x7F
                Kind::UnicodeStringConst(units.iter().map(|&u| if u > 0xFF { char::from(0x7F_u8) } else { u as u8 as char }).collect())
            }
            0x36 => Kind::StructMember(self.object()?, self.expr()?),
            0x37 => Kind::DynArrayLength(self.expr()?),
            0x38 => Kind::GlobalFunction(self.name()?, self.params()?),
            // Type conversion: 0x39 then the conversion code (Unreal 1 numbering:
            // 0x3A ByteToInt … 0x3F IntToFloat … 0x53 IntToString…), then the expression.
            0x39 => Kind::Conversion(self.u8()?, self.expr()?),
            0x40 => Kind::DynArrayInsert(self.expr()?, self.expr()?, self.expr()?),
            0x41 => Kind::DynArrayRemove(self.expr()?, self.expr()?, self.expr()?),
            0x43 => Kind::DelegateFunction(self.object()?, self.name()?, self.params()?),
            0x44 => Kind::DelegateProperty(self.name()?),
            0x45 => Kind::LetDelegate(self.expr()?, self.expr()?),
            0x60..=0x6F => {
                let index = ((token as u16 - 0x60) << 8) | self.u8()? as u16;
                Kind::NativeFunction(index, self.params()?)
            }
            0x70..=0xFF => Kind::NativeFunction(token as u16, self.params()?),
            other => return Err(ScriptError::UnknownToken(other, at)),
        };
        Ok(Expr { offset, end: self.mem, kind })
    }
}

/// Reads a complete script of `size` memory bytes.
pub fn read_script(r: &mut Reader, size: u32, none: &[i32]) -> Result<Vec<Expr>> {
    let mut s = ScriptReader::new(r, none);
    let mut statements = Vec::new();
    while s.mem < size {
        statements.push(s.read()?);
    }
    if s.mem != size {
        return Err(ScriptError::SizeMismatch { expected: size, got: s.mem });
    }
    Ok(statements)
}

/// Like `read_script`, but returns the instructions read before any error.
pub fn read_script_partial(r: &mut Reader, size: u32, none: &[i32]) -> (Vec<Expr>, Option<ScriptError>) {
    let mut s = ScriptReader::new(r, none);
    let mut statements = Vec::new();
    while s.mem < size {
        match s.read() {
            Ok(e) => statements.push(e),
            Err(e) => return (statements, Some(e)),
        }
    }
    let error = (s.mem != size).then_some(ScriptError::SizeMismatch { expected: size, got: s.mem });
    (statements, error)
}

impl Expr {
    /// Direct sub-expressions.
    pub fn children(&self) -> Vec<&Expr> {
        use Kind::*;
        match &self.kind {
            Return(a)
            | Switch(_, a)
            | JumpIfNot(_, a)
            | Assert(_, a)
            | GotoLabel(a)
            | EatString(a)
            | MetaCast(_, a)
            | Skip(_, a)
            | BoolVariable(a)
            | DynamicCast(_, a)
            | Iterator(a, _)
            | StructMember(_, a)
            | DynArrayLength(a)
            | Conversion(_, a) => vec![a],
            Case(_, a) => a.iter().map(|b| b.as_ref()).collect(),
            Let(a, b) | DynArrayElement(a, b) | LetBool(a, b) | ArrayElement(a, b) | StructCmpEq(_, a, b) | StructCmpNe(_, a, b) => vec![a, b],
            ClassContext(a, _, _, b) | Context(a, _, _, b) => vec![a, b],
            New(a, b, c, d) => vec![a, b, c, d],
            DynArrayInsert(a, b, c) | DynArrayRemove(a, b, c) => vec![a, b, c],
            LetDelegate(a, b) => vec![a, b],
            DelegateFunction(_, _, p) => p.iter().collect(),
            VirtualFunction(_, p) | FinalFunction(_, p) | GlobalFunction(_, p) | NativeFunction(_, p) => p.iter().collect(),
            _ => vec![],
        }
    }

    /// Depth-first traversal.
    pub fn walk(&self, f: &mut impl FnMut(&Expr)) {
        f(self);
        for c in self.children() {
            c.walk(f);
        }
    }
}
