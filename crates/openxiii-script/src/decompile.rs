//! Decompiler: compiled script objects to readable UnrealScript text.
//!
//! Control flow is rebuilt from the jumps: `if`/`else`, `while`
//! (`for` loops show up as `while`), `foreach`, `switch`. Whatever does not
//! structure cleanly stays as `goto` with labels.

use crate::bytecode::{Expr, Kind};
use crate::objects::{self, func, prop, Class, Function, Property, PropertyKind};
use openxiii_package::{ObjectRef, Package, Value};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;
use std::path::Path;

/// Description of a native function (index `native(N)`).
#[derive(Debug, Clone)]
pub struct Native {
    pub name: String,
    /// "Friendly" name: symbol for operators.
    pub friendly: String,
    pub flags: u32,
    pub params: usize,
}

/// Table of native functions, built by scanning script packages.
#[derive(Debug, Default, Clone)]
pub struct Natives(pub HashMap<u16, Native>);

impl Natives {
    pub fn from_dir(dir: &Path) -> Self {
        let mut table = HashMap::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Natives(table);
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("u")) {
                continue;
            }
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            let Ok(p) = Package::parse(&data) else {
                continue;
            };
            for i in 0..p.exports.len() {
                if p.export_class(i) != "Function" {
                    continue;
                }
                let Ok(f) = objects::read_function(&p, &data, i) else {
                    continue;
                };
                if f.native == 0 {
                    continue;
                }
                let params = children(&p, &data, f.body.header.children)
                    .iter()
                    .filter_map(|&c| objects::read_property(&p, &data, c).ok())
                    .filter(|pr| pr.flags & prop::PARM != 0 && pr.flags & prop::RETURN_PARM == 0)
                    .count();
                table.entry(f.native).or_insert(Native {
                    name: p.name(p.exports[i].object_name).to_string(),
                    friendly: p.name(f.body.header.friendly_name).to_string(),
                    flags: f.flags,
                    params,
                });
            }
        }
        Natives(table)
    }
}

/// Exports of a struct's Children → Next chain (file order).
pub fn children(p: &Package, data: &[u8], first: i32) -> Vec<usize> {
    let mut out = Vec::new();
    let mut current = first;
    while let ObjectRef::Export(i) = ObjectRef::from(current) {
        if out.contains(&i) || out.len() > 10_000 {
            break;
        }
        out.push(i);
        current = field_next(p, data, i);
    }
    out
}

/// `Next` field of a UField (property, function, state, enum, const, struct).
fn field_next(p: &Package, data: &[u8], i: usize) -> i32 {
    let Ok(o) = p.object_data(data, i) else {
        return 0;
    };
    let mut r = openxiii_package::Reader::new(data, o.end);
    let _ = r.compact();
    r.compact().unwrap_or(0)
}

pub struct Decompiler<'a> {
    /// Name of the decompiled package (prefix of references to its own objects).
    pub package_name: &'a str,
    pub p: &'a Package,
    pub data: &'a [u8],
    pub natives: &'a Natives,
}

fn conversion_type(code: u8) -> &'static str {
    match code {
        0x39 => "vector",
        0x3A | 0x41 | 0x44 | 0x4A => "int",
        0x3B | 0x3E | 0x45 | 0x47 | 0x48 | 0x4B | 0x4F | 0x51 => "bool",
        0x3C | 0x3F | 0x42 | 0x4C => "float",
        0x3D | 0x40 | 0x43 | 0x49 => "byte",
        0x46 | 0x5A => "name",
        0x4D => "vector",
        0x4E | 0x50 => "rotator",
        0x52..=0x59 => "string",
        _ => "cast",
    }
}

impl<'a> Decompiler<'a> {
    fn name(&self, n: i32) -> &str {
        self.p.name(n)
    }

    fn object(&self, r: i32) -> String {
        if r == 0 {
            "None".into()
        } else {
            self.p.object_name(r.into()).to_string()
        }
    }

    fn quote_object(&self, r: i32) -> String {
        if r == 0 {
            return "None".into();
        }
        let class = match ObjectRef::from(r) {
            ObjectRef::Import(i) => self.name(self.p.imports[i].class_name).to_string(),
            ObjectRef::Export(i) => self.p.export_class(i).to_string(),
            ObjectRef::None => "Object".into(),
        };
        let path = match ObjectRef::from(r) {
            ObjectRef::Export(_) => {
                format!("{}.{}", self.package_name, self.p.object_path(r.into()))
            }
            _ => self.p.object_path(r.into()),
        };
        format!("{class}'{path}'")
    }

    fn call(&self, name: &str, params: &[Expr]) -> String {
        let args: Vec<String> = params.iter().map(|e| self.expr(e)).collect();
        format!("{name}({})", args.join(", "))
    }

    /// Text of an expression.
    pub fn expr(&self, e: &Expr) -> String {
        use Kind::*;
        match &e.kind {
            LocalVariable(o) | InstanceVariable(o) | NativeParm(o) => self.object(*o),
            DefaultVariable(o) => format!("default.{}", self.object(*o)),
            Return(x) => match x.kind {
                Nothing => "return".into(),
                _ => format!("return {}", self.expr(x)),
            },
            Let(a, b) | LetBool(a, b) | LetDelegate(a, b) => {
                format!("{} = {}", self.expr(a), self.expr(b))
            }
            DynArrayElement(i, a) | ArrayElement(i, a) => {
                format!("{}[{}]", self.expr(a), self.expr(i))
            }
            New(outer, name, flags, class) => {
                let args: Vec<String> = [outer, name, flags].iter().filter(|x| !matches!(x.kind, Nothing)).map(|x| self.expr(x)).collect();
                let args = if args.is_empty() { String::new() } else { format!("({})", args.join(", ")) };
                format!("new{args} {}", self.expr(class))
            }
            ClassContext(a, _, _, b) if matches!(b.kind, DefaultVariable(_)) => {
                format!("{}.{}", self.expr(a), self.expr(b))
            }
            ClassContext(a, _, _, b) => {
                let member = self.expr(b);
                let kind = if matches!(b.kind, VirtualFunction(..) | FinalFunction(..)) { "static" } else { "default" };
                format!("{}.{kind}.{member}", self.expr(a))
            }
            Context(a, _, _, b) => format!("{}.{}", self.expr(a), self.expr(b)),
            MetaCast(c, x) => format!("class<{}>({})", self.object(*c), self.expr(x)),
            DynamicCast(c, x) => format!("{}({})", self.object(*c), self.expr(x)),
            Conversion(code, x) => format!("{}({})", conversion_type(*code), self.expr(x)),
            SelfObject => "self".into(),
            Skip(_, x) | EatString(x) | BoolVariable(x) => self.expr(x),
            VirtualFunction(n, params) => self.call(self.name(*n), params),
            FinalFunction(o, params) => self.call(&self.object(*o), params),
            GlobalFunction(n, params) => self.call(&format!("Global.{}", self.name(*n)), params),
            DelegateFunction(_, n, params) => self.call(self.name(*n), params),
            DelegateProperty(n) => self.name(*n).to_string(),
            NativeFunction(index, params) => self.native(*index, params),
            IntConst(v) => v.to_string(),
            FloatConst(v) => {
                let s = format!("{v}");
                if s.contains('.') || s.contains('e') {
                    s
                } else {
                    format!("{s}.0")
                }
            }
            StringConst(s) | UnicodeStringConst(s) => format!("{s:?}"),
            ObjectConst(o) => self.quote_object(*o),
            NameConst(n) => {
                let n = self.name(*n);
                if n.eq_ignore_ascii_case("None") {
                    "''".into()
                } else {
                    format!("'{n}'")
                }
            }
            RotationConst([p, y, r]) => format!("rot({p}, {y}, {r})"),
            VectorConst([x, y, z]) => format!("vect({x}, {y}, {z})"),
            ByteConst(b) | IntConstByte(b) => b.to_string(),
            IntZero => "0".into(),
            IntOne => "1".into(),
            True => "true".into(),
            False => "false".into(),
            NoObject => "None".into(),
            StructCmpEq(_, a, b) => format!("{} == {}", self.expr(a), self.expr(b)),
            StructCmpNe(_, a, b) => format!("{} != {}", self.expr(a), self.expr(b)),
            StructMember(m, x) => {
                let inner = self.expr(x);
                let inner = if matches!(x.kind, NativeFunction(..)) && inner.contains(' ') { format!("({inner})") } else { inner };
                format!("{inner}.{}", self.object(*m))
            }
            DynArrayLength(x) => format!("{}.Length", self.expr(x)),
            DynArrayInsert(a, i, c) => format!("{}.Insert({}, {})", self.expr(a), self.expr(i), self.expr(c)),
            DynArrayRemove(a, i, c) => format!("{}.Remove({}, {})", self.expr(a), self.expr(i), self.expr(c)),
            GotoLabel(x) => format!("goto {}", self.expr(x)),
            Assert(_, x) => format!("assert({})", self.expr(x)),
            Stop => "stop".into(),
            Nothing => String::new(),
            Iterator(x, _) => format!("foreach {}", self.expr(x)),
            IteratorPop => "/* end foreach */".into(),
            IteratorNext => "/* next */".into(),
            Jump(t) => format!("goto L{t}"),
            JumpIfNot(t, c) => format!("if(!({})) goto L{t}", self.expr(c)),
            Switch(_, x) => format!("switch({})", self.expr(x)),
            Case(_, Some(x)) => format!("case {}:", self.expr(x)),
            Case(_, None) => "default:".into(),
            LabelTable(_) => String::new(),
            EndFunctionParms => String::new(),
        }
    }

    fn native(&self, index: u16, params: &[Expr]) -> String {
        let Some(n) = self.natives.0.get(&index) else {
            return self.call(&format!("native{index}"), params);
        };
        let is_operator = n.flags & func::OPERATOR != 0;
        let paren = |e: &Expr| {
            let s = self.expr(e);
            if matches!(e.kind, Kind::NativeFunction(i, _) if self.natives.0.get(&i).is_some_and(|m| m.flags & func::OPERATOR != 0)) {
                format!("({s})")
            } else {
                s
            }
        };
        match (is_operator, params.len()) {
            (true, 2) => format!("{} {} {}", paren(&params[0]), n.friendly, paren(&params[1])),
            (true, 1) if n.flags & func::PRE_OPERATOR != 0 => {
                format!("{}{}", n.friendly, paren(&params[0]))
            }
            (true, 1) => format!("{}{}", paren(&params[0]), n.friendly),
            _ => self.call(&n.name, params),
        }
    }

    // ---- Statements and control flow ----

    /// Writes a sequence of statements; `end` is the memory offset following the slice
    /// (a jump to `end` leaves the current block).
    fn statements(&self, code: &[Expr], end: u32, out: &mut String, indent: usize, labels: &BTreeSet<u32>) {
        let pad = "\t".repeat(indent);
        let at = |offset: u32| {
            if offset == end {
                Some(code.len())
            } else {
                code.iter().position(|e| e.offset == offset)
            }
        };
        let off = |index: usize| code.get(index).map_or(end, |e| e.offset);
        let mut i = 0;
        while i < code.len() {
            let e = &code[i];
            if labels.contains(&e.offset) && i > 0 {
                let _ = writeln!(out, "{}L{}:", "\t".repeat(indent.saturating_sub(1)), e.offset);
            }
            match &e.kind {
                Kind::JumpIfNot(target, cond) => {
                    if let Some(j) = at(*target as u32).filter(|&j| j > i) {
                        if let Some(Kind::Jump(back)) = code.get(j - 1).map(|x| &x.kind) {
                            // Loop: the branch ends with a backward jump to the test.
                            if (*back as u32) <= e.offset && j - 1 > i {
                                let _ = writeln!(out, "{pad}while({})\n{pad}{{", self.expr(cond));
                                self.statements(&code[i + 1..j - 1], off(j - 1), out, indent + 1, labels);
                                let _ = writeln!(out, "{pad}}}");
                                i = j;
                                continue;
                            }
                            // if / else: the "true" branch jumps over the "false" branch.
                            if let Some(k) = at(*back as u32).filter(|&k| k >= j) {
                                let _ = writeln!(out, "{pad}if({})\n{pad}{{", self.expr(cond));
                                self.statements(&code[i + 1..j - 1], off(j - 1), out, indent + 1, labels);
                                if k > j {
                                    let _ = writeln!(out, "{pad}}}\n{pad}else\n{pad}{{");
                                    self.statements(&code[j..k], off(k), out, indent + 1, labels);
                                }
                                let _ = writeln!(out, "{pad}}}");
                                i = k;
                                continue;
                            }
                        }
                        let _ = writeln!(out, "{pad}if({})\n{pad}{{", self.expr(cond));
                        self.statements(&code[i + 1..j], off(j), out, indent + 1, labels);
                        let _ = writeln!(out, "{pad}}}");
                        i = j;
                        continue;
                    }
                }
                Kind::Iterator(x, iter_end) => {
                    if let Some(j) = at(*iter_end as u32).filter(|&j| j > i) {
                        let _ = writeln!(out, "{pad}foreach {}\n{pad}{{", self.expr(x));
                        let body_end = if matches!(code.get(j - 1).map(|x| &x.kind), Some(Kind::IteratorNext)) { j - 1 } else { j };
                        self.statements(&code[i + 1..body_end], off(body_end), out, indent + 1, labels);
                        let _ = writeln!(out, "{pad}}}");
                        i = if matches!(code.get(j).map(|x| &x.kind), Some(Kind::IteratorPop)) { j + 1 } else { j };
                        continue;
                    }
                }
                // Jump to the end of the enclosing block: already expressed by the structure.
                Kind::Jump(t) if *t as u32 == end && i + 1 == code.len() => {
                    i += 1;
                    continue;
                }
                Kind::Nothing | Kind::LabelTable(_) | Kind::EndFunctionParms => {
                    i += 1;
                    continue;
                }
                Kind::Return(x) if matches!(x.kind, Kind::Nothing) && i + 1 == code.len() && end == u32::MAX => {
                    i += 1;
                    continue;
                }
                _ => {}
            }
            let text = self.expr(e);
            if text.ends_with(':') || matches!(e.kind, Kind::Switch(..)) {
                let _ = writeln!(out, "{pad}{text}");
            } else {
                let _ = writeln!(out, "{pad}{text};");
            }
            i += 1;
        }
    }

    /// `goto` targets that do not match a recognised structure (end of an `if`
    /// with `else`, jump back to the top of a loop) and must keep a label.
    fn labels(code: &[Expr]) -> BTreeSet<u32> {
        let at = |offset: u32| code.iter().position(|e| e.offset == offset);
        let mut structured = BTreeSet::new();
        for e in code {
            if let Kind::JumpIfNot(t, _) = e.kind {
                if let Some(j) = at(t as u32) {
                    if let Some(Kind::Jump(x)) = code.get(j.wrapping_sub(1)).map(|x| &x.kind) {
                        structured.insert(*x as u32);
                    }
                }
            }
        }
        code.iter().filter_map(|e| if let Kind::Jump(t) = e.kind { Some(t as u32) } else { None }).filter(|t| !structured.contains(t)).collect()
    }

    // ---- Declarations ----

    pub fn type_name(&self, kind: &PropertyKind) -> String {
        match kind {
            PropertyKind::Byte { enumeration } if *enumeration != 0 => self.object(*enumeration),
            PropertyKind::Byte { .. } => "byte".into(),
            PropertyKind::Int => "int".into(),
            PropertyKind::Bool => "bool".into(),
            PropertyKind::Float => "float".into(),
            PropertyKind::Object { class } => self.object(*class),
            PropertyKind::Class { meta_class, .. } => {
                format!("class<{}>", self.object(*meta_class))
            }
            PropertyKind::Name => "name".into(),
            PropertyKind::Str => "string".into(),
            PropertyKind::Struct { structure } => self.object(*structure),
            PropertyKind::Array { inner } => {
                let inner = match ObjectRef::from(*inner) {
                    ObjectRef::Export(i) => objects::read_property(self.p, self.data, i).map(|p| self.type_name(&p.kind)).unwrap_or_else(|_| "?".into()),
                    _ => "?".into(),
                };
                format!("array<{inner}>")
            }
            PropertyKind::Map { .. } => "map".into(),
            PropertyKind::Delegate { function } => format!("delegate<{}>", self.object(*function)),
            PropertyKind::FixedArray { .. } => "fixedarray".into(),
        }
    }

    fn variable(&self, index: usize, property: &Property, own_class: &str) -> String {
        let name = self.name(self.p.exports[index].object_name);
        let dim = if property.array_dim > 1 { format!("[{}]", property.array_dim) } else { String::new() };
        let mut modifiers = String::new();
        for (flag, word) in [
            (prop::CONFIG, "config "),
            (prop::GLOBAL_CONFIG, "globalconfig "),
            (prop::LOCALIZED, "localized "),
            (prop::TRANSIENT, "transient "),
            (prop::TRAVEL, "travel "),
            (prop::NATIVE, "native "),
            (prop::CONST, "const "),
            (prop::EDIT_CONST, "editconst "),
            (prop::EXPORT_OBJECT, "export "),
        ] {
            if property.flags & flag != 0 {
                modifiers += word;
            }
        }
        let editable = if property.flags & prop::EDIT != 0 {
            let category = self.name(property.category);
            if category.eq_ignore_ascii_case("None") || category == own_class {
                "()".to_string()
            } else {
                format!("({category})")
            }
        } else {
            String::new()
        };
        format!("var{editable} {modifiers}{} {name}{dim};", self.type_name(&property.kind))
    }

    fn function(&self, index: usize, f: &Function, out: &mut String, indent: usize) {
        let pad = "\t".repeat(indent);
        let name = self.name(self.p.exports[index].object_name).to_string();
        let mut params = Vec::new();
        let mut locals = Vec::new();
        let mut ret = None;
        for c in children(self.p, self.data, f.body.header.children) {
            let Ok(pr) = objects::read_property(self.p, self.data, c) else {
                continue;
            };
            let pname = self.name(self.p.exports[c].object_name).to_string();
            if pr.flags & prop::RETURN_PARM != 0 {
                ret = Some(self.type_name(&pr.kind));
            } else if pr.flags & prop::PARM != 0 {
                let mut m = String::new();
                if pr.flags & prop::OPTIONAL_PARM != 0 {
                    m += "optional ";
                }
                if pr.flags & prop::OUT_PARM != 0 {
                    m += "out ";
                }
                if pr.flags & prop::COERCE_PARM != 0 {
                    m += "coerce ";
                }
                params.push(format!("{m}{} {pname}", self.type_name(&pr.kind)));
            } else {
                let dim = if pr.array_dim > 1 { format!("[{}]", pr.array_dim) } else { String::new() };
                locals.push(format!("local {} {pname}{dim};", self.type_name(&pr.kind)));
            }
        }
        let mut modifiers = String::new();
        if f.native != 0 {
            modifiers += &format!("native({}) ", f.native);
        } else if f.flags & func::NATIVE != 0 {
            modifiers += "native ";
        }
        for (flag, word) in [
            (func::STATIC, "static "),
            (func::FINAL, "final "),
            (func::SIMULATED, "simulated "),
            (func::SINGULAR, "singular "),
            (func::EXEC, "exec "),
            (func::LATENT, "latent "),
            (func::ITERATOR, "iterator "),
            (func::PRIVATE, "private "),
        ] {
            if f.flags & flag != 0 {
                modifiers += word;
            }
        }
        let keyword = if f.flags & func::OPERATOR != 0 {
            if f.flags & func::PRE_OPERATOR != 0 {
                "preoperator".to_string()
            } else if params.len() == 1 {
                "postoperator".to_string()
            } else {
                format!("operator({})", f.precedence)
            }
        } else if f.flags & func::DELEGATE != 0 {
            "delegate".into()
        } else if f.flags & func::EVENT != 0 {
            "event".into()
        } else {
            "function".into()
        };
        let shown_name = if f.flags & func::OPERATOR != 0 { self.name(f.body.header.friendly_name).to_string() } else { name };
        let ret = ret.map(|r| format!("{r} ")).unwrap_or_default();
        let _ = write!(out, "{pad}{modifiers}{keyword} {ret}{shown_name}({})", params.join(", "));
        if f.flags & func::DEFINED == 0 || (f.native != 0 && f.body.script.len() <= 2) {
            let _ = writeln!(out, ";");
            return;
        }
        let _ = writeln!(out, "\n{pad}{{");
        for l in &locals {
            let _ = writeln!(out, "{pad}\t{l}");
        }
        let labels = Self::labels(&f.body.script);
        self.statements(&f.body.script, u32::MAX, out, indent + 1, &labels);
        let _ = writeln!(out, "{pad}}}");
    }

    fn default_value(&self, v: &Value) -> String {
        match v {
            Value::Object(o) => self.quote_object(*o),
            Value::Name(n) => self.name(*n).to_string(),
            Value::Bool(b) => {
                if *b {
                    "True".into()
                } else {
                    "False".into()
                }
            }
            other => self.p.display_value(other),
        }
    }

    /// Complete text of a class.
    pub fn class(&self, index: usize, class: &Class) -> String {
        let mut out = String::new();
        let name = self.name(self.p.exports[index].object_name);
        let header = &class.state.body.header;
        let parent = if header.super_field != 0 { format!(" extends {}", self.object(header.super_field)) } else { String::new() };
        let config = self.name(class.config_name);
        let config = if config.eq_ignore_ascii_case("None") || config.eq_ignore_ascii_case("System") { String::new() } else { format!("\n\tconfig({config})") };
        let _ = writeln!(out, "// Decompiled by OpenXIII from the bytecode (not the original source).");
        let _ = writeln!(out, "class {name}{parent}{config};\n");
        let members = children(self.p, self.data, header.children);
        // Constants, enums and variables first, in declaration order
        // (the Children chain gives them in that order, but functions in reverse).
        for &c in members.iter() {
            let class_name = self.p.export_class(c);
            let member = self.name(self.p.exports[c].object_name);
            match class_name {
                "Const" => {
                    if let Ok(v) = objects::read_const(self.p, self.data, c) {
                        let _ = writeln!(out, "const {member} = {v};");
                    }
                }
                "Enum" => {
                    if let Ok(names) = objects::read_enum(self.p, self.data, c) {
                        let values: Vec<&str> = names.iter().map(|&n| self.name(n)).collect();
                        let _ = writeln!(out, "enum {member}\n{{\n\t{}\n}};\n", values.join(",\n\t"));
                    }
                }
                "Struct" => {
                    if let Ok(s) = crate::read_struct(self.p, self.data, c) {
                        let _ = writeln!(out, "struct {member}\n{{");
                        for f in children(self.p, self.data, s.header.children).iter() {
                            if let Ok(pr) = objects::read_property(self.p, self.data, *f) {
                                let _ = writeln!(out, "\t{}", self.variable(*f, &pr, member));
                            }
                        }
                        let _ = writeln!(out, "}};\n");
                    }
                }
                c2 if c2.ends_with("Property") => {
                    if let Ok(pr) = objects::read_property(self.p, self.data, c) {
                        let _ = writeln!(out, "{}", self.variable(c, &pr, name));
                    }
                }
                _ => {}
            }
        }
        let _ = writeln!(out);
        for &c in members.iter().rev() {
            match self.p.export_class(c) {
                "Function" => {
                    if let Ok(f) = objects::read_function(self.p, self.data, c) {
                        self.function(c, &f, &mut out, 0);
                        let _ = writeln!(out);
                    }
                }
                "State" => {
                    if let Ok(s) = objects::read_state(self.p, self.data, c) {
                        let sname = self.name(self.p.exports[c].object_name);
                        let editable = if s.flags & 1 != 0 { "()" } else { "" };
                        let auto = if s.flags & 2 != 0 { "auto " } else { "" };
                        let ext = if s.body.header.super_field != 0 { format!(" extends {}", self.object(s.body.header.super_field)) } else { String::new() };
                        let _ = writeln!(out, "{auto}state{editable} {sname}{ext}\n{{");
                        for f in children(self.p, self.data, s.body.header.children).iter().rev() {
                            if self.p.export_class(*f) == "Function" {
                                if let Ok(func) = objects::read_function(self.p, self.data, *f) {
                                    self.function(*f, &func, &mut out, 1);
                                }
                            }
                        }
                        let only_stop = s.body.script.len() == 1 && matches!(s.body.script[0].kind, Kind::Stop);
                        if !s.body.script.is_empty() && !only_stop {
                            let mut code_labels = BTreeSet::new();
                            let mut names = HashMap::new();
                            for e in &s.body.script {
                                if let Kind::LabelTable(t) = &e.kind {
                                    for (n, off) in t {
                                        if !self.name(*n).eq_ignore_ascii_case("None") {
                                            code_labels.insert(*off);
                                            names.insert(*off, self.name(*n).to_string());
                                        }
                                    }
                                }
                            }
                            let mut text = String::new();
                            let mut all = Self::labels(&s.body.script);
                            all.extend(code_labels.iter().copied());
                            self.statements(&s.body.script, u32::MAX, &mut text, 1, &all);
                            for (off, n) in &names {
                                text = text.replace(&format!("L{off}:"), &format!("{n}:"));
                            }
                            if let Some(first) = names.get(&0) {
                                let _ = writeln!(out, "{first}:");
                            }
                            out += &text;
                        }
                        let _ = writeln!(out, "}}\n");
                    }
                }
                _ => {}
            }
        }
        let _ = writeln!(out, "defaultproperties\n{{");
        for d in &class.defaults {
            let index = if d.array_index > 0 { format!("({})", d.array_index) } else { String::new() };
            let _ = writeln!(out, "\t{}{index}={}", self.name(d.name), self.default_value(&d.value));
        }
        let _ = writeln!(out, "}}");
        out
    }
}
