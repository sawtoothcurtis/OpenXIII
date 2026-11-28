//! Interpreter of HX sound programs (HXAudio.dll): loop, table of the 151
//! operations, operand readers, native functions
//!
//! An instruction is a 16-bit code followed by its operands. An operand starts with a
//! 16-bit tag that says where the value is: immediate (1 integer, 2 float, 3 byte,
//! 4 boolean, 5 short integer, 6 string from the table, 0x13 CUUID), object variable (7/8),
//! global variable (9/10) or actor variable (0xB/0xC) looked up by name, handler stack
//! (0xD/0xE, counted from the top), parameter passed by reference (0xF/0x12) or by value
//! (0x10), named handler (0x11). The odd codes 7/9/0xB/0xD/0xF are the writable ones.
//! An operation's destination comes before its sources.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::engine::{ActorId, Engine, ObjId, VarId};
use crate::mem::{clamp_len, Addr};
use crate::program::Program;
use std::rc::Rc;

/// Variable types (`desc + 0xC`).
pub const T_INT: u32 = 0;
pub const T_FLOAT: u32 = 1;
pub const T_STRING: u32 = 2;
pub const T_BOOL: u32 = 3;
pub const T_CHAR: u32 = 4;
/// Variable kinds (` `): 0 simple value, 3 object, 4 CUUID.
pub const K_SIMPLE: u32 = 0;
pub const K_OBJECT: u32 = 3;
pub const K_CUUID: u32 = 4;

/// Execution context of a block (HXAudio's 0x54-byte structure).
#[derive(Clone)]
pub struct Ctx {
    pub prog: Rc<Program>,
    /// + 0x38: program object being run (also the handlers' owner, + 0x04).
    pub owner: ObjId,
    /// + 0x34: the object's actor (host context), 0 if none.
    pub actor: ActorId,
    /// + 0x20: object variables.
    pub locals: Addr,
    /// + 0x24: start of the block in the code; the counter is relative to this start.
    pub code_base: u32,
    /// + 0x10 / + 0x2C: counter and end of the block.
    pub pc: u32,
    pub end: u32,
    /// + 0x0C: stack top.
    pub sp: u32,
    /// + 0x18 / + 0x30: the block's stack and its size.
    pub frame: Addr,
    pub frame_size: u32,
    /// + 0x3C: parameters received.
    pub params: Addr,
    /// + 0x40: start of the arguments pushed for the next call.
    pub argbase: u32,
    /// + 0x4C / + 0x50: access to global variables and to those of the actor.
    pub globals: bool,
    pub actor_vars: bool,
}

/// Decoded destination (the operations' 0x10-byte structure: variable or address).
#[derive(Debug, Clone, Copy, Default)]
struct Dst {
    is_var: bool,
    var: VarId,
    ptr: Addr,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Lv {
    Int,
    Float,
    Char,
    Bool,
    Str,
}

impl Lv {
    fn ty(self) -> u32 {
        match self {
            Lv::Int => T_INT,
            Lv::Float => T_FLOAT,
            Lv::Char => T_CHAR,
            Lv::Bool => T_BOOL,
            Lv::Str => T_STRING,
        }
    }
}

impl Engine {
    fn code_u16(&self, ctx: &Ctx, at: u32) -> u16 {
        let i = ctx.code_base.wrapping_add(at) as usize;
        ctx.prog.code.get(i..i + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
    }
    fn code_u32(&self, ctx: &Ctx, at: u32) -> u32 {
        let i = ctx.code_base.wrapping_add(at) as usize;
        ctx.prog.code.get(i..i + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
    }
    fn code_u8(&self, ctx: &Ctx, at: u32) -> u8 {
        ctx.prog.code.get(ctx.code_base.wrapping_add(at) as usize).copied().unwrap_or(0)
    }
    /// Tag and 32-bit value of the operand at the counter.
    fn operand(&self, ctx: &Ctx) -> (u16, u32) {
        (self.code_u16(ctx, ctx.pc), self.code_u32(ctx, ctx.pc + 2))
    }
    fn stack_addr(&self, ctx: &Ctx, off: u32) -> Addr {
        ctx.frame.wrapping_add(ctx.sp.wrapping_sub(off))
    }
    fn var_by_name(&mut self, ctx: &Ctx, actor_scope: bool, name_off: u32) -> VarId {
        let name = ctx.prog.string(name_off).to_vec();
        if actor_scope {
            if !ctx.actor_vars {
                return 0;
            }
            self.actor_var(ctx.actor, &name)
        } else {
            if !ctx.globals {
                return 0;
            }
            self.global_var(&name)
        }
    }
    /// Simple variable of type `ty`, or 0.
    fn simple_var(&self, var: VarId, ty: u32) -> VarId {
        if var != 0 && self.var_kind(var) == K_SIMPLE && self.var_type(var) == ty {
            var
        } else {
            0
        }
    }
    fn args_ptr(&self, ctx: &Ctx) -> Addr {
        if ctx.argbase < ctx.frame_size {
            ctx.frame.wrapping_add(ctx.argbase)
        } else {
            0
        }
    }

    // ----- value readers -----

    /// Integer.
    fn rd_int(&mut self, ctx: &mut Ctx) -> u32 {
        let (tag, v) = self.operand(ctx);
        ctx.pc += 6;
        match tag {
            1 => v,
            7 | 8 => self.mem.u32(ctx.locals.wrapping_add(v)),
            9 | 10 | 0xb | 0xc => {
                let var = self.var_by_name(ctx, tag >= 0xb, v);
                let var = self.simple_var(var, T_INT);
                if var == 0 {
                    return 0;
                }
                self.var_read_notify(var);
                self.var_u32(var)
            }
            0xd | 0xe => self.mem.u32(self.stack_addr(ctx, v)),
            0xf | 0x12 => self.ref_read_u32(ctx, v, T_INT),
            0x10 => self.mem.u32(ctx.params.wrapping_add(v)),
            _ => 0,
        }
    }

    /// Parameter passed by reference: {kind 1 = address, 2 = variable; pointer}.
    fn ref_read_u32(&mut self, ctx: &Ctx, off: u32, ty: u32) -> u32 {
        let r = ctx.params.wrapping_add(off);
        match self.mem.u8(r) {
            1 => {
                let p = self.mem.u32(r + 4);
                self.mem.u32(p)
            }
            2 => {
                let var = self.simple_var(self.mem.u32(r + 4), ty);
                if var == 0 {
                    return 0;
                }
                self.var_read_notify(var);
                self.var_u32(var)
            }
            _ => 0,
        }
    }

    /// Float: same rules as the integer, type 1.
    fn rd_float(&mut self, ctx: &mut Ctx) -> f32 {
        let (tag, v) = self.operand(ctx);
        ctx.pc += 6;
        let bits = match tag {
            2 => v,
            7 | 8 => self.mem.u32(ctx.locals.wrapping_add(v)),
            9 | 10 | 0xb | 0xc => {
                let var = self.var_by_name(ctx, tag >= 0xb, v);
                let var = self.simple_var(var, T_FLOAT);
                if var == 0 {
                    0
                } else {
                    self.var_read_notify(var);
                    self.var_u32(var)
                }
            }
            0xd | 0xe => self.mem.u32(self.stack_addr(ctx, v)),
            0xf | 0x12 => self.ref_read_u32(ctx, v, T_FLOAT),
            0x10 => self.mem.u32(ctx.params.wrapping_add(v)),
            _ => 0,
        };
        f32::from_bits(bits)
    }

    /// Character: 1-byte immediate; default value "space" (0x20).
    fn rd_char(&mut self, ctx: &mut Ctx) -> u8 {
        let (tag, v) = self.operand(ctx);
        if tag == 3 {
            ctx.pc += 3;
            return v as u8;
        }
        ctx.pc += 6;
        match tag {
            7 | 8 => self.mem.u8(ctx.locals.wrapping_add(v)),
            9 | 10 | 0xb | 0xc => {
                let var = self.var_by_name(ctx, tag >= 0xb, v);
                let var = self.simple_var(var, T_CHAR);
                if var == 0 {
                    return 0x20;
                }
                self.var_read_notify(var);
                self.var_u32(var) as u8
            }
            0xd | 0xe => self.mem.u8(self.stack_addr(ctx, v)),
            0xf | 0x12 => {
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        let p = self.mem.u32(r + 4);
                        self.mem.u8(p)
                    }
                    2 => {
                        let var = self.simple_var(self.mem.u32(r + 4), T_CHAR);
                        if var == 0 {
                            return 0x20;
                        }
                        self.var_read_notify(var);
                        self.var_u32(var) as u8
                    }
                    _ => 0x20,
                }
            }
            0x10 => self.mem.u8(ctx.params.wrapping_add(v)),
            _ => 0x20,
        }
    }

    /// Boolean: true if the byte read is not zero.
    fn rd_bool(&mut self, ctx: &mut Ctx) -> bool {
        self.rd_bool_raw(ctx) != 0
    }

    /// Boolean as the original stores it: 0 or 1 for an immediate, a local variable, the stack, a
    /// parameter or an address reference (setne), but the variable's byte (+ 0x2C) as is for a named variable
    /// or a kind-2 reference, which push (0x4B), copy (0x62) and equality (0x91, 0x92) keep.
    fn rd_bool_raw(&mut self, ctx: &mut Ctx) -> u8 {
        let (tag, v) = self.operand(ctx);
        if tag == 4 {
            ctx.pc += 3;
            return (v as u8 != 0) as u8;
        }
        ctx.pc += 6;
        match tag {
            7 | 8 => (self.mem.u8(ctx.locals.wrapping_add(v)) != 0) as u8,
            9 | 10 | 0xb | 0xc => {
                let var = self.var_by_name(ctx, tag >= 0xb, v);
                let var = self.simple_var(var, T_BOOL);
                if var == 0 {
                    return 0;
                }
                self.var_read_notify(var);
                self.var_u32(var) as u8
            }
            0xd | 0xe => (self.mem.u8(self.stack_addr(ctx, v)) != 0) as u8,
            0xf | 0x12 => {
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        let p = self.mem.u32(r + 4);
                        (self.mem.u8(p) != 0) as u8
                    }
                    2 => {
                        let var = self.simple_var(self.mem.u32(r + 4), T_BOOL);
                        if var == 0 {
                            return 0;
                        }
                        self.var_read_notify(var);
                        self.var_u32(var) as u8
                    }
                    _ => 0,
                }
            }
            0x10 => (self.mem.u8(ctx.params.wrapping_add(v)) != 0) as u8,
            _ => 0,
        }
    }

    /// Short integer: 2-byte immediate; no named variable.
    fn rd_short(&mut self, ctx: &mut Ctx) -> u16 {
        let (tag, v) = self.operand(ctx);
        if tag == 5 {
            ctx.pc += 4;
            return v as u16;
        }
        ctx.pc += 6;
        match tag {
            7 | 8 => self.mem.u16(ctx.locals.wrapping_add(v)),
            0xd | 0xe => self.mem.u16(self.stack_addr(ctx, v)),
            0xf | 0x12 => {
                let r = ctx.params.wrapping_add(v);
                if self.mem.u8(r) == 1 {
                    let p = self.mem.u32(r + 4);
                    self.mem.u16(p)
                } else {
                    0
                }
            }
            0x10 => self.mem.u16(ctx.params.wrapping_add(v)),
            _ => 0,
        }
    }

    /// Raw word (pushed by operation 0x4C).
    fn rd_raw(&mut self, ctx: &mut Ctx) -> u32 {
        let (tag, v) = self.operand(ctx);
        ctx.pc += 6;
        match tag {
            7 | 8 => self.mem.u32(ctx.locals.wrapping_add(v)),
            0xd | 0xe => self.mem.u32(self.stack_addr(ctx, v)),
            0x10 => self.mem.u32(ctx.params.wrapping_add(v)),
            _ => 0,
        }
    }

    /// Object.
    fn rd_obj(&mut self, ctx: &mut Ctx) -> ObjId {
        let (tag, v) = self.operand(ctx);
        ctx.pc += 6;
        match tag {
            7 | 8 => self.mem.u32(ctx.locals.wrapping_add(v)),
            9 | 10 | 0xb | 0xc => {
                let var = self.var_by_name(ctx, tag >= 0xb, v);
                if var != 0 && self.var_kind(var) == K_OBJECT {
                    self.var_read_notify(var);
                    self.var_u32(var)
                } else {
                    0
                }
            }
            0xd | 0xe => self.mem.u32(self.stack_addr(ctx, v)),
            0xf | 0x12 => {
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        let p = self.mem.u32(r + 4);
                        self.mem.u32(p)
                    }
                    2 => {
                        let var = self.mem.u32(r + 4);
                        self.var_read_notify(var);
                        self.var_u32(var)
                    }
                    _ => 0,
                }
            }
            0x10 => self.mem.u32(ctx.params.wrapping_add(v)),
            _ => 0,
        }
    }

    /// CUUID. Reproduces the original's quirks: the stack only advances the counter by
    /// 2 bytes, and the reference reads its second word through the next parameter.
    fn rd_cuuid(&mut self, ctx: &mut Ctx) -> [u32; 2] {
        let (tag, v) = self.operand(ctx);
        let null = [0u32, 0];
        match tag {
            7 | 8 => {
                ctx.pc += 6;
                let a = ctx.locals.wrapping_add(v);
                [self.mem.u32(a), self.mem.u32(a + 4)]
            }
            9 | 10 | 0xb | 0xc => {
                ctx.pc += 6;
                let var = self.var_by_name(ctx, tag >= 0xb, v);
                if var != 0 && self.var_kind(var) == K_CUUID {
                    self.var_cuuid(var)
                } else {
                    null
                }
            }
            0xd | 0xe => {
                let a = self.stack_addr(ctx, v);
                ctx.pc += 2;
                [self.mem.u32(a), self.mem.u32(a + 4)]
            }
            0xf | 0x12 => {
                ctx.pc += 6;
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        let p = self.mem.u32(r + 4);
                        let q = self.mem.u32(r + 8);
                        [self.mem.u32(p), self.mem.u32(q)]
                    }
                    2 => {
                        let var = self.mem.u32(r + 4);
                        if var != 0 && self.var_kind(var) == K_CUUID {
                            self.var_cuuid(var)
                        } else {
                            null
                        }
                    }
                    _ => null,
                }
            }
            0x10 => {
                ctx.pc += 6;
                let a = ctx.params.wrapping_add(v);
                [self.mem.u32(a), self.mem.u32(a + 4)]
            }
            0x13 => {
                let hi = self.code_u32(ctx, ctx.pc + 6);
                ctx.pc += 10;
                [v, hi]
            }
            _ => {
                ctx.pc += 6;
                null
            }
        }
    }

    /// String: text, empty by default.
    fn rd_str(&mut self, ctx: &mut Ctx) -> Vec<u8> {
        let (tag, v) = self.operand(ctx);
        ctx.pc += 6;
        let text = match tag {
            6 => ctx.prog.string(v).to_vec(),
            7 | 8 => {
                let s = self.mem.u32(ctx.locals.wrapping_add(v));
                self.mem.string(s)
            }
            9 | 10 | 0xb | 0xc => {
                let var = self.var_by_name(ctx, tag >= 0xb, v);
                let var = self.simple_var(var, T_STRING);
                if var == 0 {
                    Vec::new()
                } else {
                    self.var_read_notify(var);
                    self.var_string(var)
                }
            }
            0xd | 0xe => {
                let s = self.mem.u32(self.stack_addr(ctx, v));
                self.mem.string(s)
            }
            0xf | 0x12 => {
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        let p = self.mem.u32(r + 4);
                        let s = self.mem.u32(p);
                        self.mem.string(s)
                    }
                    2 => {
                        let var = self.simple_var(self.mem.u32(r + 4), T_STRING);
                        if var == 0 {
                            Vec::new()
                        } else {
                            self.var_read_notify(var);
                            self.var_string(var)
                        }
                    }
                    _ => Vec::new(),
                }
            }
            0x10 => {
                let s = self.mem.u32(ctx.params.wrapping_add(v));
                self.mem.string(s)
            }
            _ => Vec::new(),
        };
        clamp_len(&text).to_vec()
    }

    // ----- destinations -----

    /// Destination of a simple type (integer, float
    /// character, boolean, string). An unknown tag does not advance
    /// the counter (as in the original).
    fn dst(&mut self, ctx: &mut Ctx, lv: Lv) -> (Dst, bool) {
        let (tag, v) = self.operand(ctx);
        let mut d = Dst::default();
        match tag {
            7 => {
                ctx.pc += 6;
                d.ptr = ctx.locals.wrapping_add(v);
                (d, true)
            }
            9 | 0xb => {
                ctx.pc += 6;
                let accessor = if tag == 9 { ctx.globals } else { ctx.actor_vars };
                if !accessor {
                    return (d, false);
                }
                d.is_var = true;
                let var = self.var_by_name(ctx, tag == 0xb, v);
                d.var = self.simple_var(var, lv.ty());
                let ok = d.var != 0;
                (d, ok)
            }
            0xd => {
                ctx.pc += 6;
                d.ptr = self.stack_addr(ctx, v);
                (d, true)
            }
            0xf => {
                ctx.pc += 6;
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        d.ptr = self.mem.u32(r + 4);
                        let ok = if lv == Lv::Str { self.mem.u32(d.ptr) != 0 } else { d.ptr != 0 };
                        (d, ok)
                    }
                    2 => {
                        // The original boolean checks the character type here.
                        let ty = if lv == Lv::Bool { T_CHAR } else { lv.ty() };
                        d.var = self.simple_var(self.mem.u32(r + 4), ty);
                        d.is_var = d.var != 0;
                        let ok = d.is_var;
                        (d, ok)
                    }
                    _ => (d, false),
                }
            }
            _ => {
                if lv == Lv::Str {
                    ctx.pc += 6;
                }
                (d, false)
            }
        }
    }

    fn put_u32(&mut self, d: Dst, value: u32) {
        if !d.is_var {
            if d.ptr != 0 {
                self.mem.set_u32(d.ptr, value);
            }
        } else if d.var != 0 {
            self.set_var_u32(d.var, value);
            self.var_write_notify(d.var);
        }
    }
    fn put_u8(&mut self, d: Dst, value: u8) {
        if !d.is_var {
            if d.ptr != 0 {
                self.mem.set_u8(d.ptr, value);
            }
        } else if d.var != 0 {
            // Only the low byte of the value (+ 0x2C) is written.
            let prior = self.var_u32(d.var);
            self.set_var_u32(d.var, (prior & !0xFF) | value as u32);
            self.var_write_notify(d.var);
        }
    }
    fn put_f32(&mut self, d: Dst, value: f32) {
        self.put_u32(d, value.to_bits());
    }
    /// String write: the slot must already hold a string object.
    fn put_str(&mut self, d: Dst, text: &[u8]) {
        let text = clamp_len(text);
        if !d.is_var {
            if d.ptr != 0 {
                let s = self.mem.u32(d.ptr);
                if s != 0 {
                    self.mem.set_string(s, text);
                }
            }
        } else if d.var != 0 {
            self.set_var_string(d.var, text);
            self.var_write_notify(d.var);
        }
    }

    /// Read-write of a character: destination and current value.
    fn rmw_char(&mut self, ctx: &mut Ctx) -> (Dst, bool, u8) {
        let (tag, v) = self.operand(ctx);
        let mut d = Dst::default();
        match tag {
            7 => {
                ctx.pc += 6;
                d.ptr = ctx.locals.wrapping_add(v);
                (d, true, self.mem.u8(d.ptr))
            }
            0xd => {
                ctx.pc += 6;
                d.ptr = self.stack_addr(ctx, v);
                (d, true, self.mem.u8(d.ptr))
            }
            9 | 0xb => {
                ctx.pc += 6;
                let accessor = if tag == 9 { ctx.globals } else { ctx.actor_vars };
                if !accessor {
                    return (d, false, 0x20);
                }
                d.is_var = true;
                let var = self.var_by_name(ctx, tag == 0xb, v);
                d.var = self.simple_var(var, T_CHAR);
                if d.var == 0 {
                    return (d, false, 0x20);
                }
                self.var_read_notify(d.var);
                let cur = self.var_u32(d.var) as u8;
                (d, true, cur)
            }
            0xf => {
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        d.ptr = self.mem.u32(r + 4);
                        ctx.pc += 6;
                        let cur = if d.ptr == 0 { 0x20 } else { self.mem.u8(d.ptr) };
                        (d, d.ptr != 0, cur)
                    }
                    2 => {
                        ctx.pc += 6;
                        d.var = self.simple_var(self.mem.u32(r + 4), T_CHAR);
                        d.is_var = d.var != 0;
                        // The original does not read the current value in this case (left undefined).
                        (d, d.is_var, 0)
                    }
                    _ => (d, false, 0),
                }
            }
            _ => (d, false, 0x20),
        }
    }

    /// Read-write of a word (integer, float).
    fn rmw_u32(&mut self, ctx: &mut Ctx, ty: u32) -> (Dst, bool, u32) {
        let (tag, v) = self.operand(ctx);
        let mut d = Dst::default();
        match tag {
            7 => {
                ctx.pc += 6;
                d.ptr = ctx.locals.wrapping_add(v);
                (d, true, self.mem.u32(d.ptr))
            }
            0xd => {
                ctx.pc += 6;
                d.ptr = self.stack_addr(ctx, v);
                (d, true, self.mem.u32(d.ptr))
            }
            9 | 0xb => {
                ctx.pc += 6;
                let accessor = if tag == 9 { ctx.globals } else { ctx.actor_vars };
                if !accessor {
                    return (d, false, 0);
                }
                d.is_var = true;
                let var = self.var_by_name(ctx, tag == 0xb, v);
                d.var = self.simple_var(var, ty);
                if d.var == 0 {
                    return (d, false, 0);
                }
                self.var_read_notify(d.var);
                let cur = self.var_u32(d.var);
                (d, true, cur)
            }
            0xf => {
                // Neither kind 1 nor 2: refused without advancing the counter.
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        ctx.pc += 6;
                        d.ptr = self.mem.u32(r + 4);
                        // Float: the value read is the pointer itself (fld [params + v + 4]).
                        let cur = if ty == T_FLOAT {
                            d.ptr
                        } else if d.ptr == 0 {
                            0
                        } else {
                            self.mem.u32(d.ptr)
                        };
                        (d, d.ptr != 0, cur)
                    }
                    2 => {
                        ctx.pc += 6;
                        d.var = self.simple_var(self.mem.u32(r + 4), ty);
                        d.is_var = d.var != 0;
                        // The original does not read the variable's value (integer,
                        // float): the output slot keeps its initial content.
                        (d, d.is_var, 0)
                    }
                    _ => (d, false, 0),
                }
            }
            _ => (d, false, 0),
        }
    }

    /// Read-write of a string.
    fn rmw_str(&mut self, ctx: &mut Ctx) -> (Dst, bool, Vec<u8>) {
        let (tag, v) = self.operand(ctx);
        let mut d = Dst::default();
        match tag {
            7 | 0xd => {
                ctx.pc += 6;
                d.ptr = if tag == 7 { ctx.locals.wrapping_add(v) } else { self.stack_addr(ctx, v) };
                let s = self.mem.u32(d.ptr);
                (d, true, self.mem.string(s))
            }
            9 | 0xb => {
                ctx.pc += 6;
                let accessor = if tag == 9 { ctx.globals } else { ctx.actor_vars };
                if !accessor {
                    return (d, false, Vec::new());
                }
                d.is_var = true;
                let var = self.var_by_name(ctx, tag == 0xb, v);
                d.var = self.simple_var(var, T_STRING);
                if d.var == 0 {
                    return (d, false, Vec::new());
                }
                self.var_read_notify(d.var);
                let cur = self.var_string(d.var);
                (d, true, cur)
            }
            0xf => {
                // Neither kind 1 nor 2: refused without advancing the counter.
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        ctx.pc += 6;
                        // The destination is the parameter's slot itself,
                        // the value read that of the string pointed to.
                        d.ptr = r + 4;
                        let p = self.mem.u32(d.ptr);
                        // The parameter holds the string object itself.
                        (d, p != 0, self.mem.string(p))
                    }
                    2 => {
                        ctx.pc += 6;
                        d.var = self.simple_var(self.mem.u32(r + 4), T_STRING);
                        d.is_var = d.var != 0;
                        // The original does not read the variable's value.
                        (d, d.is_var, Vec::new())
                    }
                    _ => (d, false, Vec::new()),
                }
            }
            _ => {
                // The original also consumes unknown tags.
                ctx.pc += 6;
                (d, false, Vec::new())
            }
        }
    }

    /// Object destination.
    fn dst_obj(&mut self, ctx: &mut Ctx) -> (Dst, bool) {
        let (tag, v) = self.operand(ctx);
        let mut d = Dst::default();
        match tag {
            7 => {
                ctx.pc += 6;
                d.ptr = ctx.locals.wrapping_add(v);
                (d, true)
            }
            0xd => {
                ctx.pc += 6;
                d.ptr = self.stack_addr(ctx, v);
                (d, true)
            }
            9 | 0xb => {
                ctx.pc += 6;
                let accessor = if tag == 9 { ctx.globals } else { ctx.actor_vars };
                if !accessor {
                    return (Dst::default(), false);
                }
                let var = self.var_by_name(ctx, tag == 0xb, v);
                if var != 0 && self.var_kind(var) == K_OBJECT {
                    (Dst { is_var: true, var, ptr: 0 }, true)
                } else {
                    (Dst::default(), false)
                }
            }
            0xf => {
                ctx.pc += 6;
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        d.ptr = self.mem.u32(r + 4);
                        (d, d.ptr != 0)
                    }
                    2 => {
                        let var = self.mem.u32(r + 4);
                        if var != 0 && self.var_kind(var) == K_OBJECT {
                            (Dst { is_var: true, var, ptr: 0 }, true)
                        } else {
                            (Dst::default(), false)
                        }
                    }
                    _ => (Dst::default(), false),
                }
            }
            _ => (Dst::default(), false),
        }
    }

    /// Object assignment: references held by the actor, count of objects
    /// held by the program (object + 0x50).
    fn put_obj(&mut self, ctx: &Ctx, d: Dst, value: ObjId) {
        if !d.is_var {
            let old = if d.ptr != 0 { self.mem.u32(d.ptr) } else { 0 };
            if value != 0 {
                self.actor_addref(ctx.actor, value);
                if old == 0 {
                    self.program_held(ctx.owner, 1);
                }
            }
            if old != 0 {
                self.actor_release(ctx.actor, old);
                if value == 0 {
                    self.program_held(ctx.owner, -1);
                }
            }
            if d.ptr != 0 {
                self.mem.set_u32(d.ptr, value);
            }
        } else if d.var != 0 {
            self.set_var_u32(d.var, value);
            self.var_write_notify(d.var);
        }
    }

    /// CUUID destination; the write does not notify the variable.
    fn dst_cuuid(&mut self, ctx: &mut Ctx) -> (Dst, bool) {
        let (tag, v) = self.operand(ctx);
        let mut d = Dst::default();
        match tag {
            7 => {
                ctx.pc += 6;
                d.ptr = ctx.locals.wrapping_add(v);
                (d, true)
            }
            0xd => {
                ctx.pc += 6;
                d.ptr = self.stack_addr(ctx, v);
                (d, true)
            }
            9 | 0xb => {
                ctx.pc += 6;
                let accessor = if tag == 9 { ctx.globals } else { ctx.actor_vars };
                if !accessor {
                    return (d, false);
                }
                let var = self.var_by_name(ctx, tag == 0xb, v);
                if var != 0 && self.var_kind(var) == K_CUUID {
                    (Dst { is_var: true, var, ptr: 0 }, true)
                } else {
                    (Dst { is_var: true, var: 0, ptr: 0 }, false)
                }
            }
            0xf => {
                ctx.pc += 6;
                let r = ctx.params.wrapping_add(v);
                match self.mem.u8(r) {
                    1 => {
                        d.ptr = self.mem.u32(r + 4);
                        (d, d.ptr != 0)
                    }
                    2 => {
                        let var = self.mem.u32(r + 4);
                        if var != 0 && self.var_kind(var) == K_CUUID {
                            (Dst { is_var: true, var, ptr: 0 }, true)
                        } else {
                            (d, false)
                        }
                    }
                    _ => (d, false),
                }
            }
            _ => (d, false),
        }
    }

    fn put_cuuid(&mut self, d: Dst, value: [u32; 2]) {
        if !d.is_var {
            if d.ptr != 0 {
                self.mem.set_u32(d.ptr, value[0]);
                self.mem.set_u32(d.ptr + 4, value[1]);
            }
        } else if d.var != 0 {
            self.set_var_cuuid(d.var, value);
        }
    }

    /// Address of an object or stack variable (strings 0x65/0x66).
    fn slot_addr(&mut self, ctx: &mut Ctx) -> Addr {
        let (tag, v) = self.operand(ctx);
        ctx.pc += 6;
        match tag {
            7 => ctx.locals.wrapping_add(v),
            0xd => self.stack_addr(ctx, v),
            _ => 0,
        }
    }

    fn push_u32(&mut self, ctx: &mut Ctx, value: u32) {
        let a = ctx.frame.wrapping_add(ctx.sp);
        self.mem.set_u32(a, value);
        ctx.sp += 4;
    }

    // ----- execution -----

    /// Runs the block described by `ctx` to its end.
    pub fn run(&mut self, ctx: &mut Ctx) {
        let mut guard = 0u32;
        while ctx.pc < ctx.end {
            let op = self.code_u16(ctx, ctx.pc);
            self.step(ctx, op);
            guard += 1;
            if guard > 5_000_000 {
                // Guard against an infinite loop (the original game would hang).
                // Log text of the original HXAudio.dll, kept as it is.
                self.log(format!("programme {} : boucle interrompue", ctx.prog.cuuid));
                break;
            }
        }
    }

    /// One operation (table). Original handlers by code:
    /// 00…8F (26), 03, 04, 05, 06,
    /// 07, 08, 09, 0A, 0B, 0C,
    /// 0D, 0E, 10, 11, 12, 13,
    /// 14, 15, 16, 17, 18, 19,
    /// 1A, 1B, 1C, 1D, 1E, 1F,
    /// 20, 21, 22, 23, 24, 25,
    /// 26, 27, 28, 29, 2A, 2B,
    /// 2C, 2D, 2E, 2F, 30, 31,
    /// 32, 33, 34, 35, 36, 37,
    /// 38, 39, 3A, 3B, 3C, 3D,
    /// 3E, 45, 46, 47, 48, 49,
    /// 4A, 4B, 4C, 4D, 4E, 55,
    /// 56, 57, 58, 59, 5A, 5B,
    /// 5C, 5D, 5E, 5F, 60, 61,
    /// 62, 63, 64, 65, 66, 6B/6C,
    /// 6D, 6E, 6F, 70, 71, 72,
    /// 73, 74, 75, 76, 77, 78,
    /// 79, 7A, 7B, 7C, 7D, 7E,
    /// 7F, 80/87, 81, 82, 83, 88,
    /// 89, 8A, 8B, 8C, 90, 91,
    /// 92, 93, 94, 95, 96
    fn step(&mut self, ctx: &mut Ctx, op: u16) {
        let at = ctx.pc;
        match op {
            // Missing operations: end of the block.
            0 | 1 | 2 | 0xf | 0x3f..=0x44 | 0x4f..=0x54 | 0x67..=0x6a | 0x84..=0x86 | 0x8d..=0x8f => {
                ctx.pc = ctx.end + 2;
            }
            // Jumps relative to the instruction.
            3 => {
                ctx.pc += 2;
                let off = self.rd_int(ctx);
                ctx.pc = off.wrapping_add(at);
            }
            4 | 5 => {
                ctx.pc += 2;
                let off = self.rd_int(ctx);
                let cond = self.rd_bool(ctx);
                if cond == (op == 4) {
                    ctx.pc = off.wrapping_add(at);
                }
            }
            6 => self.one_int(ctx, |a| a.wrapping_neg()),
            7 => self.one_float(ctx, |a| -a),
            8 => self.one_char(ctx, |a| a.wrapping_neg()),
            9 => self.one_int(ctx, |a| !a),
            0xa => self.one_char(ctx, |a| !a),
            0xb => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Bool);
                let a = self.rd_bool(ctx);
                if ok {
                    self.put_u8(d, (!a) as u8);
                }
            }
            0xc => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Bool);
                let a = self.rd_int(ctx);
                if ok {
                    self.put_u8(d, (a == 0) as u8);
                }
            }
            0xd => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Bool);
                let a = self.rd_float(ctx);
                if ok {
                    // Two x87 comparisons: true if a < -1e-38 or a > 1e-38; NaN gives false.
                    self.put_u8(d, (a < -1e-38f32 || a > 1e-38f32) as u8);
                }
            }
            0xe => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Bool);
                let a = self.rd_char(ctx);
                if ok {
                    self.put_u8(d, (a != 0) as u8);
                }
            }
            0x10 => self.one_int(ctx, |a| a),
            0x11 => self.one_char(ctx, |a| a),
            0x12 => self.one_float(ctx, |a| a),
            0x13 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Str);
                let a = self.rd_str(ctx);
                if ok {
                    self.put_str(d, &a);
                }
            }
            // Arithmetic: integer, float, character (and string concatenation).
            0x14 => self.bin_int(ctx, |a, b| a.wrapping_add(b)),
            0x15 => self.bin_float(ctx, |a, b| a + b),
            0x16 => self.bin_char(ctx, |a, b| a.wrapping_add(b)),
            0x17 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Str);
                let mut a = self.rd_str(ctx);
                let b = self.rd_str(ctx);
                if ok {
                    a.extend_from_slice(clamp_len(&b));
                    self.put_str(d, &a);
                }
            }
            0x18 => self.bin_int(ctx, |a, b| a.wrapping_mul(b)),
            0x19 => self.bin_float(ctx, |a, b| a * b),
            0x1a => self.bin_char(ctx, |a, b| a.wrapping_mul(b)),
            0x1b => self.bin_int(ctx, |a, b| a.wrapping_sub(b)),
            0x1c => self.bin_float(ctx, |a, b| a - b),
            0x1d => self.bin_char(ctx, |a, b| a.wrapping_sub(b)),
            0x1e => self.bin_int(ctx, |a, b| if b != 0 { (a as i32).wrapping_div(b as i32) as u32 } else { 0x7fff7fff }),
            0x1f => self.bin_float(ctx, |a, b| if b != 0.0 { a / b } else { f32::from_bits(0x7149f2ca) }),
            0x20 => self.bin_char(ctx, |a, b| if b != 0 { a / b } else { 0xff }),
            // Comparisons: signed integer, unsigned short and character, float, string.
            0x21..=0x3e => self.compare(ctx, op),
            // Pushes.
            0x45 => {
                let a = self.code_u32(ctx, at + 2);
                let b = self.code_u32(ctx, at + 6);
                ctx.pc += 10;
                self.push_u32(ctx, a);
                self.push_u32(ctx, b);
            }
            0x46 => {
                ctx.pc += 2;
                let v = self.rd_cuuid(ctx);
                self.push_u32(ctx, v[0]);
                self.push_u32(ctx, v[1]);
            }
            0x47 => {
                ctx.pc += 2;
                let v = self.rd_int(ctx);
                self.push_u32(ctx, v);
            }
            0x48 => {
                ctx.pc += 2;
                let v = self.rd_short(ctx);
                self.mem.set_u16(ctx.frame.wrapping_add(ctx.sp), v);
                ctx.sp += 2;
            }
            0x49 => {
                ctx.pc += 2;
                let v = self.rd_float(ctx);
                self.push_u32(ctx, v.to_bits());
            }
            0x4a => {
                ctx.pc += 2;
                let v = self.rd_char(ctx);
                self.mem.set_u8(ctx.frame.wrapping_add(ctx.sp), v);
                ctx.sp += 1;
            }
            0x4b => {
                ctx.pc += 2;
                let v = self.rd_bool_raw(ctx);
                self.mem.set_u8(ctx.frame.wrapping_add(ctx.sp), v);
                ctx.sp += 1;
            }
            0x4c => {
                ctx.pc += 2;
                let v = self.rd_raw(ctx);
                self.push_u32(ctx, v);
            }
            0x4d => {
                ctx.pc += 2;
                if self.code_u16(ctx, ctx.pc) == 0x11 {
                    let off = self.code_u32(ctx, ctx.pc + 2);
                    ctx.pc = at + 8;
                    let name = ctx.prog.string(off).to_vec();
                    let h = self.handler_ref(ctx.owner, &name);
                    self.actor_addref(ctx.actor, ctx.owner);
                    self.mem.set_u32(ctx.frame.wrapping_add(ctx.sp), h);
                    ctx.sp += 8;
                } else {
                    ctx.pc += 6;
                }
            }
            0x4e => {
                ctx.pc += 2;
                let v = self.rd_obj(ctx);
                self.push_u32(ctx, v);
            }
            0x55 => self.bin_bool(ctx, |a, b| a || b),
            0x56 => self.bin_bool(ctx, |a, b| a && b),
            0x57 => self.bin_bool(ctx, |a, b| a != b),
            0x58 => self.bin_int(ctx, |a, b| a | b),
            0x59 => self.bin_char(ctx, |a, b| a | b),
            0x5a => self.bin_int(ctx, |a, b| a & b),
            0x5b => self.bin_char(ctx, |a, b| a & b),
            0x5c => self.bin_int(ctx, |a, b| a ^ b),
            0x5d => self.bin_char(ctx, |a, b| a ^ b),
            0x5e => {
                // Short integer: object variable, stack or reference.
                ctx.pc += 2;
                let (tag, v) = self.operand(ctx);
                ctx.pc += 6;
                let (ptr, ok) = match tag {
                    7 => (ctx.locals.wrapping_add(v), true),
                    0xd => (self.stack_addr(ctx, v), true),
                    0xf => {
                        let r = ctx.params.wrapping_add(v);
                        if self.mem.u8(r) == 1 {
                            let p = self.mem.u32(r + 4);
                            (p, p != 0)
                        } else {
                            (0, false)
                        }
                    }
                    _ => (0, false),
                };
                let value = self.rd_short(ctx);
                if ok && ptr != 0 {
                    self.mem.set_u16(ptr, value);
                }
            }
            0x5f => {
                ctx.pc += 2;
                let (d, ok) = self.dst_obj(ctx);
                let value = self.rd_obj(ctx);
                if ok {
                    self.put_obj(ctx, d, value);
                }
            }
            0x60 => {
                ctx.pc += 2;
                let n = self.rd_int(ctx);
                self.mem.fill(ctx.frame.wrapping_add(ctx.sp), n as usize, 0);
                ctx.sp = ctx.sp.wrapping_add(n);
            }
            0x61 => {
                ctx.pc += 2;
                let n = self.rd_int(ctx);
                ctx.sp = ctx.sp.wrapping_sub(n);
            }
            0x62 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Bool);
                let a = self.rd_bool_raw(ctx);
                if ok {
                    self.put_u8(d, a);
                }
            }
            0x63 => {
                // Call of an object's handler by name.
                ctx.pc += 2;
                let obj = self.rd_obj(ctx);
                let name_off = self.code_u32(ctx, ctx.pc);
                ctx.pc += 4;
                if obj == 0 {
                    return;
                }
                let msg = if obj != ctx.owner { 0x3ef } else { 0x3e9 };
                let args = self.args_ptr(ctx);
                self.send_named(obj, msg, ctx.prog.string(name_off).to_vec().as_slice(), args);
            }
            0x64 => {
                ctx.pc += 2;
                let (d, ok) = self.dst_obj(ctx);
                if ok {
                    self.put_obj(ctx, d, 0);
                }
            }
            0x65 => {
                ctx.pc += 2;
                let a = self.slot_addr(ctx);
                if a != 0 {
                    let s = self.mem.new_string(b"");
                    self.mem.set_u32(a, s);
                }
            }
            0x66 => {
                ctx.pc += 2;
                let a = self.slot_addr(ctx);
                if a != 0 {
                    let s = self.mem.u32(a);
                    if s != 0 {
                        self.mem.delete_string(s);
                        self.mem.set_u32(a, 0);
                    }
                }
            }
            0x6b | 0x6c => {
                // Call of a program function.
                let index = self.code_u32(ctx, at + 2);
                ctx.pc += 6;
                self.call_function(ctx, index);
            }
            0x6d => {
                // Call of a handler of the program itself.
                let name_off = self.code_u32(ctx, at + 2);
                ctx.pc += 6;
                let args = self.args_ptr(ctx);
                let owner = ctx.owner;
                self.send_named(owner, 0x3e9, ctx.prog.string(name_off).to_vec().as_slice(), args);
            }
            0x6e => {
                // Message to an object: arguments on the stack.
                ctx.pc += 2;
                let obj = self.rd_obj(ctx);
                let msg = self.rd_int(ctx);
                if obj != 0 {
                    let args = ctx.frame.wrapping_add(ctx.argbase);
                    self.send(obj, msg, args);
                }
            }
            0x6f => {
                ctx.pc += 2;
                let n = self.rd_int(ctx);
                ctx.argbase = ctx.sp.wrapping_sub(n);
            }
            0x70 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Float);
                let a = self.rd_int(ctx);
                if ok {
                    self.put_f32(d, a as i32 as f32);
                }
            }
            0x71 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Str);
                let a = self.rd_int(ctx);
                if ok {
                    self.put_str(d, format!("{}", a as i32).as_bytes());
                }
            }
            0x72 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Str);
                let a = self.rd_char(ctx);
                if ok {
                    let text: &[u8] = if a == 0 { b" " } else { &[a] };
                    self.put_str(d, text);
                }
            }
            0x73 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Int);
                let a = self.rd_char(ctx);
                if ok {
                    self.put_u32(d, a as u32);
                }
            }
            0x74 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Float);
                let a = self.rd_char(ctx);
                if ok {
                    self.put_f32(d, a as f32);
                }
            }
            0x75 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Str);
                let a = self.rd_float(ctx);
                if ok {
                    self.put_str(d, msvc_f3(a).as_bytes());
                }
            }
            0x76 | 0x77 => {
                ctx.pc += 2;
                let id = self.rd_int(ctx);
                let args = self.args_ptr(ctx);
                self.native(ctx, id, args);
            }
            0x78 | 0x7c => {
                ctx.pc += 2;
                let (d, ok, cur) = self.rmw_char(ctx);
                let b = self.rd_char(ctx);
                if ok {
                    let v = if op == 0x78 { cur.wrapping_add(b) } else { cur.wrapping_sub(b) };
                    self.put_u8(d, v);
                }
            }
            0x79 | 0x7d => {
                ctx.pc += 2;
                let (d, ok, cur) = self.rmw_u32(ctx, T_INT);
                let b = self.rd_int(ctx);
                if ok {
                    let v = if op == 0x79 { cur.wrapping_add(b) } else { cur.wrapping_sub(b) };
                    self.put_u32(d, v);
                }
            }
            0x7a | 0x7e => {
                ctx.pc += 2;
                let (d, ok, cur) = self.rmw_u32(ctx, T_FLOAT);
                let b = self.rd_float(ctx);
                if ok {
                    let cur = f32::from_bits(cur);
                    let v = if op == 0x7a { op_float(b, cur, |x, y| x + y) } else { op_float(cur, b, |x, y| x - y) };
                    self.put_f32(d, v);
                }
            }
            0x7b => {
                ctx.pc += 2;
                let (d, ok, mut cur) = self.rmw_str(ctx);
                let b = self.rd_str(ctx);
                if ok {
                    cur.extend_from_slice(clamp_len(&b));
                    self.put_str(d, &cur);
                }
            }
            0x7f => {
                // Call by name on the actor (actor vtable+0x10).
                let name_off = self.code_u32(ctx, at + 2);
                ctx.pc += 6;
                let args = self.args_ptr(ctx);
                if ctx.actor != 0 {
                    let name = ctx.prog.string(name_off).to_vec();
                    self.actor_call(ctx.actor, &name, args);
                }
            }
            0x80 | 0x87 => ctx.pc = ctx.end + 0x100,
            0x81 => {
                let n = self.code_u32(ctx, at + 2);
                ctx.pc += 6;
                self.mem.fill(ctx.frame.wrapping_add(ctx.sp), n as usize, 0);
                ctx.sp = ctx.sp.wrapping_add(n);
            }
            0x82 => {
                let n = self.code_u32(ctx, at + 2);
                ctx.pc += 6;
                ctx.sp = ctx.sp.wrapping_sub(n);
            }
            0x83 => {
                let n = self.code_u32(ctx, at + 2);
                ctx.pc += 6;
                ctx.argbase = ctx.sp.wrapping_sub(n);
            }
            0x88 | 0x89 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, if op == 0x88 { Lv::Int } else { Lv::Float });
                let v = self.code_u32(ctx, ctx.pc);
                ctx.pc += 4;
                if ok {
                    self.put_u32(d, v);
                }
            }
            0x8a => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Bool);
                let v = self.code_u8(ctx, ctx.pc + 2);
                ctx.pc += 1;
                if ok {
                    self.put_u8(d, (v != 0) as u8);
                }
            }
            0x8b => {
                ctx.pc += 2;
                let (d, ok) = self.dst_cuuid(ctx);
                let v = [self.code_u32(ctx, ctx.pc), self.code_u32(ctx, ctx.pc + 4)];
                ctx.pc += 8;
                if ok {
                    self.put_cuuid(d, v);
                }
            }
            0x8c => self.copy_op(ctx, at),
            0x90 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Str);
                let a = self.rd_bool(ctx);
                if ok {
                    self.put_str(d, if a { b"true" } else { b"false" });
                }
            }
            0x91 | 0x92 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Bool);
                let a = self.rd_bool_raw(ctx);
                let b = self.rd_bool_raw(ctx);
                if ok {
                    self.put_u8(d, ((a == b) == (op == 0x91)) as u8);
                }
            }
            0x93 | 0x94 => {
                ctx.pc += 2;
                let (d, ok) = self.dst(ctx, Lv::Bool);
                let a = self.rd_obj(ctx);
                if ok {
                    self.put_u8(d, ((a == 0) == (op == 0x93)) as u8);
                }
            }
            0x95 => {
                ctx.pc += 2;
                let (d, ok) = self.dst_cuuid(ctx);
                let v = self.rd_cuuid(ctx);
                if ok {
                    self.put_cuuid(d, v);
                }
            }
            0x96 => self.push_ref(ctx, at),
            // Operations outside the table: the table only has 151 entries.
            _ => {
                *self.unhandled.entry(("operation", op as u32)).or_default() += 1;
                ctx.pc = ctx.end + 2
            }
        }
    }

    fn one_int(&mut self, ctx: &mut Ctx, f: impl Fn(u32) -> u32) {
        ctx.pc += 2;
        let (d, ok) = self.dst(ctx, Lv::Int);
        let a = self.rd_int(ctx);
        if ok {
            self.put_u32(d, f(a));
        }
    }
    fn one_float(&mut self, ctx: &mut Ctx, f: impl Fn(f32) -> f32) {
        ctx.pc += 2;
        let (d, ok) = self.dst(ctx, Lv::Float);
        let a = self.rd_float(ctx);
        if ok {
            self.put_f32(d, f(a));
        }
    }
    fn one_char(&mut self, ctx: &mut Ctx, f: impl Fn(u8) -> u8) {
        ctx.pc += 2;
        let (d, ok) = self.dst(ctx, Lv::Char);
        let a = self.rd_char(ctx);
        if ok {
            self.put_u8(d, f(a));
        }
    }
    fn bin_int(&mut self, ctx: &mut Ctx, f: impl Fn(u32, u32) -> u32) {
        ctx.pc += 2;
        let (d, ok) = self.dst(ctx, Lv::Int);
        let a = self.rd_int(ctx);
        let b = self.rd_int(ctx);
        if ok {
            self.put_u32(d, f(a, b));
        }
    }
    fn bin_float(&mut self, ctx: &mut Ctx, f: impl Fn(f32, f32) -> f32) {
        ctx.pc += 2;
        let (d, ok) = self.dst(ctx, Lv::Float);
        let a = self.rd_float(ctx);
        let b = self.rd_float(ctx);
        if ok {
            // Two NaNs: the x87 returns the one with the larger mantissa (Rust's SSE returns the first).
            let v = op_float(a, b, f);
            self.put_f32(d, v);
        }
    }
    fn bin_char(&mut self, ctx: &mut Ctx, f: impl Fn(u8, u8) -> u8) {
        ctx.pc += 2;
        let (d, ok) = self.dst(ctx, Lv::Char);
        let a = self.rd_char(ctx);
        let b = self.rd_char(ctx);
        if ok {
            self.put_u8(d, f(a, b));
        }
    }
    fn bin_bool(&mut self, ctx: &mut Ctx, f: impl Fn(bool, bool) -> bool) {
        ctx.pc += 2;
        let (d, ok) = self.dst(ctx, Lv::Bool);
        let a = self.rd_bool(ctx);
        let b = self.rd_bool(ctx);
        if ok {
            self.put_u8(d, f(a, b) as u8);
        }
    }

    /// Comparisons 0x21..0x3E: six relations × (integer, short, float, character, string).
    fn compare(&mut self, ctx: &mut Ctx, op: u16) {
        use std::cmp::Ordering;
        let rel = (op - 0x21) / 5;
        let kind = (op - 0x21) % 5;
        ctx.pc += 2;
        let (d, ok) = self.dst(ctx, Lv::Bool);
        let ord: Option<Ordering> = match kind {
            0 => {
                let a = self.rd_int(ctx) as i32;
                let b = self.rd_int(ctx) as i32;
                Some(a.cmp(&b))
            }
            1 => {
                let a = self.rd_short(ctx);
                let b = self.rd_short(ctx);
                Some(a.cmp(&b))
            }
            2 => {
                let a = self.rd_float(ctx);
                let b = self.rd_float(ctx);
                a.partial_cmp(&b)
            }
            3 => {
                let a = self.rd_char(ctx);
                let b = self.rd_char(ctx);
                Some(a.cmp(&b))
            }
            _ => {
                let a = self.rd_str(ctx);
                let b = self.rd_str(ctx);
                Some(a.cmp(&b))
            }
        };
        let r = match (rel, ord) {
            (0, Some(o)) => o == Ordering::Equal,
            (1, Some(o)) => o != Ordering::Equal,
            (2, Some(o)) => o == Ordering::Less,
            (3, Some(o)) => o != Ordering::Greater,
            (4, Some(o)) => o == Ordering::Greater,
            (5, Some(o)) => o != Ordering::Less,
            // Unordered floats: the original x87 tests give false, except "not equal".
            (1, None) => true,
            _ => false,
        };
        if ok || kind == 0 && rel == 1 {
            self.put_u8(d, r as u8);
        }
    }

    /// Copy of a word: into an object variable or the stack, from the object, the
    /// stack or a named handler.
    fn copy_op(&mut self, ctx: &mut Ctx, at: u32) {
        ctx.pc = at + 2;
        let tag = self.code_u16(ctx, ctx.pc);
        let off = self.code_u32(ctx, ctx.pc + 2);
        let dst = match tag {
            7 => ctx.locals.wrapping_add(off),
            0xd => self.stack_addr(ctx, off),
            _ => {
                ctx.pc += 0xc;
                return;
            }
        };
        ctx.pc = at + 8;
        let stag = self.code_u16(ctx, at + 8);
        let soff = self.code_u32(ctx, at + 10);
        match stag {
            7 | 8 => {
                let v = self.mem.u32(ctx.locals.wrapping_add(soff));
                self.mem.set_u32(dst, v);
                ctx.pc += 6;
            }
            0xd | 0xe => {
                let v = self.mem.u32(self.stack_addr(ctx, soff));
                self.mem.set_u32(dst, v);
                ctx.pc += 6;
            }
            0x11 => {
                ctx.pc = at + 0xe;
                let name = ctx.prog.string(soff).to_vec();
                let h = self.handler_ref(ctx.owner, &name);
                self.mem.set_u32(dst, h);
            }
            _ => ctx.pc += 6,
        }
    }

    /// Push of a reference {kind, pointer}.
    fn push_ref(&mut self, ctx: &mut Ctx, at: u32) {
        ctx.pc = at + 2;
        let tag = self.code_u16(ctx, ctx.pc);
        let v = self.code_u32(ctx, ctx.pc + 2);
        let slot = ctx.frame.wrapping_add(ctx.sp);
        match tag {
            7 => {
                self.mem.set_u32(slot, 1);
                self.mem.set_u32(slot + 4, ctx.locals.wrapping_add(v));
            }
            9 | 0xb => {
                ctx.pc = at + 8;
                let var = self.var_by_name(ctx, tag == 0xb, v);
                if var != 0 {
                    self.mem.set_u32(slot, 2);
                    self.mem.set_u32(slot + 4, var);
                } else {
                    self.mem.set_u32(slot, 1);
                    self.mem.set_u32(slot + 4, 0);
                }
                ctx.sp += 8;
                return;
            }
            0xd => {
                self.mem.set_u32(slot, 1);
                self.mem.set_u32(slot + 4, self.stack_addr(ctx, v));
            }
            0xf => {
                let r = ctx.params.wrapping_add(v);
                let (a, b) = (self.mem.u32(r), self.mem.u32(r + 4));
                self.mem.set_u32(slot, a);
                self.mem.set_u32(slot + 4, b);
            }
            _ => {
                self.mem.set_u32(slot, 1);
                self.mem.set_u32(slot + 4, 0);
            }
        }
        ctx.sp += 8;
        ctx.pc += 6;
    }

    /// Runs a program function in a nested context.
    fn call_function(&mut self, ctx: &mut Ctx, index: u32) {
        let Some(f) = ctx.prog.functions.get(index as usize).copied() else {
            return;
        };
        let frame = self.mem.alloc(f.stack as usize);
        #[cfg(test)]
        {
            self.last_frame = frame;
        }
        let params = if ctx.argbase < ctx.frame_size { ctx.frame.wrapping_add(ctx.argbase) } else { 0 };
        let mut sub = Ctx {
            prog: ctx.prog.clone(),
            owner: ctx.owner,
            actor: ctx.actor,
            locals: ctx.locals,
            code_base: f.start,
            pc: 0,
            end: f.len,
            sp: 0,
            frame,
            frame_size: f.stack,
            params,
            // The nested context restarts with the start of the arguments at zero (structure zeroed,
            // only copies the caller's start of arguments into + 0x14 for the parameters).
            argbase: 0,
            globals: ctx.globals,
            actor_vars: ctx.actor_vars,
        };
        self.run(&mut sub);
        self.mem.release(frame);
    }

    /// Native functions: `args` points to the pushed arguments.
    fn native(&mut self, ctx: &mut Ctx, id: u32, args: Addr) {
        match id {
            1 => {
                // Random integer: (rand() × n) / 32767 truncated.
                let (d, ok) = self.dst(ctx, Lv::Int);
                let n = self.mem.u32(args) as i32;
                // FILD, FIMUL, FMUL (24 bits), then _ftol.
                let r = self.rand();
                let v = (((r as f64 * n as f64) as f32) * f32::from_bits(0x38000100)) as i32;
                if ok {
                    self.put_u32(d, v as u32);
                }
            }
            2 => {
                // Random float between two bounds.
                let (d, ok) = self.dst(ctx, Lv::Float);
                if ok {
                    let lo = self.mem.f32(args);
                    let hi = self.mem.f32(args + 4);
                    let r = self.rand();
                    // ((high − low) × rand) × (1/32767) + low, on 24 bits with an extended exponent: no
                    // overflow before storing as float.
                    let (hi, lo) = (f64::from(hi), f64::from(lo));
                    let p = x87_24(x87_24(x87_24(hi - lo) * f64::from(r)) * f64::from(f32::from_bits(0x38000100)));
                    let v = x87_24(p + lo) as f32;
                    self.put_f32(d, v);
                }
            }
            3 => {
                // Debug trace of a string.
                let s = self.mem.u32(args);
                let text = if s == 0 { b"<NULL>".to_vec() } else { self.mem.string(s) };
                // Log text of the original HXAudio.dll, kept as it is.
                self.log(format!("programme {} : {}", ctx.prog.cuuid, String::from_utf8_lossy(&text)));
            }
            4 => {
                // Creates a sound object from a CUUID and attaches it to the actor.
                if args == 0 {
                    return;
                }
                let id = [self.mem.u32(args), self.mem.u32(args + 4)];
                let (d, ok) = self.dst_obj(ctx);
                if !ok {
                    return;
                }
                match self.create_object_for(ctx.actor, openxiii_assets::hx::Cuuid(id[0], id[1])) {
                    Some(obj) => {
                        self.actor_attach(ctx.actor, obj, ctx.owner, true);
                        self.put_obj(ctx, d, obj);
                        self.actor_release(ctx.actor, obj);
                    }
                    None => self.put_obj(ctx, d, 0),
                }
            }
            5 | 7 | 9 | 10 | 13 | 15..=19 | 21 => {
                let (d, ok) = self.dst(ctx, Lv::Float);
                if ok {
                    let (a, b) = (self.mem.u32(args), self.mem.u32(args + 4));
                    if let Some(v) = native_float(id, a, b) {
                        self.put_f32(d, v);
                    }
                }
            }
            6 | 8 | 14 => {
                let (d, ok) = self.dst(ctx, Lv::Int);
                if ok {
                    let (a, b) = (self.mem.u32(args), self.mem.u32(args + 4));
                    if let Some(v) = native_integer(id, a, b) {
                        self.put_u32(d, v as u32);
                    }
                }
            }
            11 | 12 => {}
            20 => {
                let (d, ok) = self.dst_obj(ctx);
                if ok {
                    let owner = ctx.owner;
                    self.put_obj(ctx, d, owner);
                }
            }
            22 => {
                let (d, _) = self.dst(ctx, Lv::Int);
                let t = self.time_ms();
                self.put_u32(d, t);
            }
            23 => {
                let (d, _) = self.dst(ctx, Lv::Int);
                let s = self.mem.u32(args);
                let text = self.mem.string(s);
                self.put_u32(d, atoi(&text) as u32);
            }
            24..=26 => {
                // Left, right and middle of a string.
                let s = self.mem.u32(args);
                let text = self.mem.string(s);
                let len = text.len() & 0xffff;
                let n = self.mem.u32(args + 4) as usize;
                let part: Vec<u8> = match id {
                    24 => text[..n.min(len)].to_vec(),
                    25 => {
                        let k = n.min(len);
                        text[len - k..len].to_vec()
                    }
                    _ => {
                        let start = n.min(len);
                        let count = (self.mem.u32(args + 8) as usize).min(len - start);
                        text[start..start + count].to_vec()
                    }
                };
                let (d, _) = self.dst(ctx, Lv::Str);
                self.put_str(d, &part);
            }
            _ => *self.unhandled.entry(("native", id)).or_default() += 1,
        }
    }
}

/// msvcr70's `sprintf("%.3f", (double)a)`: unsigned zero, infinities and NaN written as the CRT does
/// ("1.#IO", "1.#QO", "-1.#IO" for the indefinite 0xFFC00000: three digits of precision).
fn msvc_f3(a: f32) -> String {
    let bits = a.to_bits();
    let sign = if bits >> 31 != 0 { "-" } else { "" };
    if a.is_nan() {
        return format!("{sign}1.#{}O", if bits == 0xffc0_0000 { 'I' } else { 'Q' });
    }
    if a.is_infinite() {
        return format!("{sign}1.#IO");
    }
    if a == 0.0 {
        return "0.000".to_string();
    }
    format!("{:.3}", a as f64)
}

/// x87 float operation, NaN included (the x87 rule, not that of Rust's SSE, which depends on the operand order
/// chosen at compile time): a single NaN (or none) lets `f` run (the NaN comes out quiet in both
/// cases, and `f` keeps its special cases, like division by zero); a signalling and a quiet one give the quiet one; two of the same kind give the one with
/// the larger mantissa, the positive one for equal mantissas.
fn op_float(a: f32, b: f32, f: impl Fn(f32, f32) -> f32) -> f32 {
    const SILENCER: u32 = 0x40_0000;
    let (ba, bb) = (a.to_bits(), b.to_bits());
    match (a.is_nan(), b.is_nan()) {
        (true, false) | (false, true) | (false, false) => f(a, b),
        (true, true) => {
            let (qa, qb) = (ba & SILENCER != 0, bb & SILENCER != 0);
            let takes_b = if qa != qb {
                qb
            } else {
                let (ma, mb) = (ba & 0x7f_ffff, bb & 0x7f_ffff);
                mb > ma || (mb == ma && bb >> 31 < ba >> 31)
            };
            f32::from_bits(if takes_b { bb } else { ba } | SILENCER)
        }
    }
}

/// Float natives of HX programs (…), on the raw words of their first two
/// arguments: 5 absolute value, 7 remainder (zero if the divisor is), 9 integer → float, 10 semitones
/// → ratio, 13 integer part, 15 cosine, 16 sine, 17 and 18 decimal and natural logarithms (zero
/// for a value ≤ 0), 19 exponential, 21 power.
pub(crate) fn native_float(id: u32, a: u32, b: u32) -> Option<f32> {
    let (fa, fb) = (f32::from_bits(a), f32::from_bits(b));
    Some(match id {
        5 => {
            if fa < 0.0 {
                -fa
            } else {
                fa
            }
        }
        7 => {
            // The CRT returns +0 for a zero dividend and a finite divisor.
            if fb == 0.0 || (fa == 0.0 && fb.is_finite()) {
                0.0
            } else {
                (fa as f64 % fb as f64) as f32
            }
        }
        9 => (a as i32) as f32,
        10 => semitones_to_ratio(fa),
        13 => (fa as f64).floor() as f32,
        15 => (fa as f64).dcos() as f32,
        16 => (fa as f64).dsin() as f32,
        17 | 18 => {
            let x = fa as f64;
            (if x <= 0.0 {
                0.0
            } else if id == 17 {
                x.dlog10()
            } else {
                x.dln()
            }) as f32
        }
        // FLDL2E, FMUL, FRNDINT, F2XM1, FLD1, FADDP, FSCALE: see `listener::exp_x87`.
        19 => crate::listener::exp_x87(fa),
        21 => (fa as f64).dpowf(fb as f64) as f32,
        _ => return None,
    })
}

/// Integer natives: 6 absolute value, 8 remainder (zero if the divisor is), 14 integer part of a float.
pub(crate) fn native_integer(id: u32, a: u32, b: u32) -> Option<i32> {
    Some(match id {
        6 => (a as i32).wrapping_abs(),
        8 => {
            if b as i32 == 0 {
                0
            } else {
                (a as i32).wrapping_rem(b as i32)
            }
        }
        // floor (CRT) then _ftol: 64 bits truncated, low bits kept (0x80…0 out of range).
        14 => {
            let v = (f32::from_bits(a) as f64).floor();
            if v.abs() < 9.223372036854775807e18 {
                (v as i64) as i32
            } else {
                0
            }
        }
        _ => return None,
    })
}

/// Semitones to frequency ratio (native 10): table of the 12 semitones and
/// series expansion of the remainder, then doubling per octave.
pub fn semitones_to_ratio(x: f32) -> f32 {
    const TABLE: [f32; 12] = [
        1.0,
        1.0594631433486938,
        1.1224620342254639,
        1.1892070770263672,
        1.2599210739135742,
        1.3348398208618164,
        1.4142135381698608,
        1.4983071088790894,
        1.587401032447815,
        1.6817928552627563,
        1.7817974090576172,
        1.8877485990524292,
    ];
    // 24-bit FPU on the game thread: fmod and floor exact, products and sums rounded to single.
    let neg = x < 0.0;
    let a = if neg { -x } else { x };
    let m = (a as f64) % 12.0;
    let fl = m.floor();
    // floor(x × (1/12)): product by the double constant rounded to 24 bits, then _ftol.
    let octaves = (((a as f64 * (1.0 / 12.0)) as f32) as f64).floor() as i32;
    let k = (fl as i32).clamp(0, 11) as usize;
    let t = TABLE[k];
    let u = (((m - fl) * 0.05776226504666211) as f32) * t;
    let mut r = t + ((0.5 * u) + 1.0) * u;
    if neg {
        r = 1.0 / r;
        for _ in 0..octaves {
            r *= 0.5;
        }
    } else {
        for _ in 0..octaves {
            r += r;
        }
    }
    r
}

fn atoi(text: &[u8]) -> i32 {
    let mut i = 0;
    while i < text.len() && (text[i] == b' ' || (9..=13).contains(&text[i])) {
        i += 1;
    }
    let mut neg = false;
    if i < text.len() && (text[i] == b'-' || text[i] == b'+') {
        neg = text[i] == b'-';
        i += 1;
    }
    let mut v: i32 = 0;
    while i < text.len() && text[i].is_ascii_digit() {
        v = v.wrapping_mul(10).wrapping_add((text[i] - b'0') as i32);
        i += 1;
    }
    if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

/// Result of an x87 operation in 24-bit precision: mantissa rounded to 24 bits (to
/// nearest, even), extended exponent (no float overflow).
fn x87_24(x: f64) -> f64 {
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

