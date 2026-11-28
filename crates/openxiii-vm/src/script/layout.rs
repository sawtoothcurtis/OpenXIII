//! Memory layout of objects in the original engine (Unreal 2 C++ classes): position of
//! each script property, computed as UStruct::Link does (alignments, booleans grouped into 32-bit
//! bit fields). Used to read the machine code of the game's DLLs, which accesses properties by
//! their position (`[esi + 0x2b4]` = Pawn.Health…).

use crate::types::{PropDef, Type};
use crate::value::ClassId;
use crate::Vm;

#[derive(Debug, Clone)]
pub struct FieldLayout {
    pub owner: String,
    pub name: String,
    pub offset: usize,
    pub size: usize,
    /// For a boolean: mask of the bit in its 32-bit word.
    pub bit: Option<u32>,
    pub ty: String,
}

fn align(x: usize, a: usize) -> usize {
    x.div_ceil(a) * a
}

impl Vm {
    /// (size, alignment) of a value of this type.
    pub(crate) fn type_size(&mut self, ty: &Type) -> (usize, usize) {
        {
            use crate::mutant;
        }
        match ty {
            Type::Byte => (1, 1),
            Type::Int | Type::Float | Type::Name | Type::Object(_) | Type::Class(_) | Type::Bool | Type::Unknown => (4, 4),
            // FString and TArray: pointer, count, capacity.
            Type::Str | Type::Array(_) => (12, 4),
            // Delegate: object + function name.
            Type::Delegate => (8, 4),
            Type::Struct(name) => {
                // struct_def includes the fields of the parent structs (Plane = Vector + W).
                let fields: Vec<PropDef> = self.struct_def(name).map(|s| s.fields.clone()).unwrap_or_default();
                let (_, size) = self.lay_out(&fields, 0, "");
                // UStruct::Link rounds PropertiesSize to 4 bytes; UStructProperty::Link
                // aligns to 4, or to 1 for an empty struct.
                let size = align(size, 4);
                let size = size;
                (size, if size >= 4 { 4 } else { 1 })
            }
        }
    }

    /// Places the properties from `start`; returns their positions and the final size.
    pub(crate) fn lay_out(&mut self, props: &[PropDef], start: usize, owner: &str) -> (Vec<FieldLayout>, usize) {
        let mut out = Vec::new();
        let mut offset = start;
        let mut bool_word: Option<(usize, u32)> = None;
        for p in props {
            if matches!(p.ty, Type::Bool) {
                match bool_word {
                    Some((at, bit)) if bit < 0x8000_0000 => {
                        let bit = bit << 1;
                        bool_word = Some((at, bit));
                        out.push(FieldLayout { owner: owner.into(), name: p.name.clone(), offset: at, size: 4, bit: Some(bit), ty: "bool".into() });
                    }
                    _ => {
                        offset = align(offset, 4);
                        bool_word = Some((offset, 1));
                        out.push(FieldLayout { owner: owner.into(), name: p.name.clone(), offset, size: 4, bit: Some(1), ty: "bool".into() });
                        offset += 4;
                    }
                }
                continue;
            }
            bool_word = None;
            let (size, a) = self.type_size(&p.ty);
            offset = align(offset, a);
            let ty = match &p.ty {
                Type::Byte => "byte".to_string(),
                Type::Int => "int".into(),
                Type::Float => "float".into(),
                Type::Name => "name".into(),
                Type::Str => "string".into(),
                Type::Object(c) => format!("object<{c}>"),
                Type::Class(c) => format!("class<{c}>"),
                Type::Struct(s) => format!("struct {s}"),
                Type::Array(_) => "array".into(),
                Type::Delegate => "delegate".into(),
                Type::Bool => "bool".into(),
                Type::Unknown => "?".into(),
            };
            out.push(FieldLayout { owner: owner.into(), name: p.name.clone(), offset, size: size * p.dim, bit: None, ty });
            offset += size * p.dim;
        }
        (out, offset)
    }

    /// Full layout of a class (inherited properties included) and its size.
    pub fn class_layout(&mut self, class: &str) -> Option<(Vec<FieldLayout>, usize)> {
        match self.class(class) {
            Some(c) => self.class_layout_of(c),
            None => Some((Vec::new(), 0)),
        }
    }

    /// Full layout of class `ci` (two classes from different packages can have the same name).
    pub fn class_layout_of(&mut self, ci: ClassId) -> Option<(Vec<FieldLayout>, usize)> {
        let mut chain = Vec::new();
        let mut c = Some(ci);
        while let Some(ci) = c {
            chain.push(ci);
            c = self.classes[ci].super_class;
        }
        chain.reverse();
        let mut all = Vec::new();
        let mut size = 0;
        let mut inherited = 0;
        for ci in chain {
            // Properties specific to the class: those that follow the inherited ones.
            let own: Vec<PropDef> = self.classes[ci].props[inherited..].to_vec();
            inherited = self.classes[ci].props.len();
            let name = self.classes[ci].name.clone();
            // Each class starts on a 4-byte boundary.
            let (fields, end) = self.lay_out(&own, align(size, 4), &name);
            all.extend(fields);
            size = end;
        }
        let size = size;
        Some((all, align(size, 4)))
    }
}

impl Vm {
    /// Property tree of a class (`oxiii proptree`):
    /// one line per property (name, kind, position, element size, ArrayDim, PropertyFlags,
    /// boolean mask, struct or array element), then each struct used.
    pub fn prop_tree(&mut self, class: &str) -> Option<String> {
        let ci = self.class(class)?;
        let props = self.classes[ci].props.clone();
        let (fields, size) = self.class_layout(class)?;
        let mut out = format!("class {class} {size}\n");
        let mut structs: Vec<String> = Vec::new();
        self.prop_tree_lines(&props, &fields, &mut out, &mut structs);
        let mut i = 0;
        while i < structs.len() {
            let s = structs[i].clone();
            i += 1;
            let fields_def: Vec<PropDef> = self.struct_def(&s).map(|d| d.fields.clone()).unwrap_or_default();
            let (lay, _) = self.lay_out(&fields_def, 0, "");
            let (size, _) = self.type_size(&Type::Struct(s.clone()));
            // Fields inherited from the parent struct (at the head of `fields`): TFieldIterator reads those of
            // the struct first, then those of the parent (SerializeBin: Plane = W then X, Y, Z).
            let inherited_value = self.struct_def(&s).and_then(|d| d.super_struct.clone()).and_then(|p| self.struct_def(&p).map(|d| d.fields.len())).unwrap_or(0);
            out += &format!("struct {s} {size} {inherited_value}\n");
            self.prop_tree_lines(&fields_def, &lay, &mut out, &mut structs);
        }
        Some(out)
    }

    /// "C name package flags ini parent inherited": the package, the class flags (hexadecimal), the
    /// configuration file, the parent class ("-" if none) and the number of inherited properties.
    pub fn class_info(&mut self, class: &str) -> Option<String> {
        let ci = self.class(class)?;
        let c = &self.classes[ci];
        let sup = c.super_class;
        let inherited = sup.map_or(0, |s| self.classes[s].props.len());
        Some(format!(
            "C {} {} {:x} {} {} {} {}
",
            c.name,
            self.packages[c.package].name,
            c.flags,
            c.config_name,
            sup.map_or("-".to_string(), |s| self.classes[s].name.clone()),
            inherited,
            self.class_within.get(&ci).cloned().unwrap_or_else(|| "-".into())
        ))
    }

    /// `exec` functions declared by a class, for ScriptConsoleExec
    /// (`oxiii funcinfo`): "F class name flags parameter size", then one "P" line per
    /// parameter (like `prop_tree`, flags CPF_Parm 0x80, OutParm 0x100, OptionalParm 0x10, CoerceParm 0x40),
    /// then each struct used.
    pub fn func_info(&mut self, class: &str) -> Option<String> {
        self.func_info_gen(class, false)
    }

    /// Like `func_info`, for all the functions of the class (events, functions with a return value): the return value is
    /// the last parameter (CPF_Parm 0x80, OutParm 0x100, ReturnParm 0x400) and the F line adds NumParms and ReturnValueOffset
    /// (255 without a return value), as UFunction::Serialize derives them from walking the parameters.
    pub fn func_info_all(&mut self, class: &str) -> Option<String> {
        self.func_info_gen(class, true)
    }

    fn func_info_gen(&mut self, class: &str, everything: bool) -> Option<String> {
        let ci = self.class(class)?;
        let mut names_list: Vec<(String, usize)> = self.classes[ci].functions.iter().map(|(k, &f)| (k.clone(), f)).collect();
        names_list.sort();
        let mut out = String::new();
        let mut structs: Vec<String> = Vec::new();
        for (_, f) in names_list {
            let fd = &self.functions[f];
            if (!everything && fd.flags & openxiii_script::objects::func::EXEC == 0) || fd.owner != ci {
                continue;
            }
            let ret = fd.ret.clone();
            let mut params: Vec<PropDef> = fd
                .params
                .iter()
                .filter_map(|p| {
                    fd.locals.get(p.local).cloned().map(|mut d| {
                        d.flags = 0x80 | if p.out { 0x100 } else { 0 } | if p.optional { 0x10 } else { 0 } | if p.coerce { 0x40 } else { 0 };
                        d
                    })
                })
                .collect();
            if let (true, Some(ty)) = (everything, ret) {
                params.push(PropDef { name: "ReturnValue".into(), ty, dim: 1, flags: 0x80 | 0x100 | 0x400, inner_flags: 0, enumeration: None, public: true });
            }
            let (name_value, flags) = (fd.name.clone(), fd.flags);
            let (lay, size) = self.lay_out(&params, 0, "");
            let supplement = if everything {
                let return_value = params.iter().position(|p| p.flags & 0x400 != 0).map_or(255, |i| lay[i].offset & 0xFF);    // ReturnValueOffset is a byte in the original (TcpLink.ReadBinary: 260 becomes 4)
                format!(" {} {return_value}", params.len())
            } else {
                String::new()
            };
            out += &format!("F {} {} {:x} {}{supplement}
", self.classes[ci].name, name_value, flags, lay.last().map_or(0, |l| l.offset + l.size));
            let _ = size;    // ParmsSize of UFunction::Serialize: end of the last parameter, without alignment
            self.prop_tree_lines(&params, &lay, &mut out, &mut structs);
        }
        let mut i = 0;
        while i < structs.len() {
            let s = structs[i].clone();
            i += 1;
            let fields_def: Vec<PropDef> = self.struct_def(&s).map(|d| d.fields.clone()).unwrap_or_default();
            let (lay, _) = self.lay_out(&fields_def, 0, "");
            let (size, _) = self.type_size(&Type::Struct(s.clone()));
            let inherited_value = self.struct_def(&s).and_then(|d| d.super_struct.clone()).and_then(|p| self.struct_def(&p).map(|d| d.fields.len())).unwrap_or(0);
            out += &format!("struct {s} {size} {inherited_value}
");
            self.prop_tree_lines(&fields_def, &lay, &mut out, &mut structs);
        }
        Some(out)
    }

    fn prop_tree_lines(&mut self, props: &[PropDef], fields: &[FieldLayout], out: &mut String, structs: &mut Vec<String>) {
        for (p, f) in props.iter().zip(fields) {
            let elem = f.size / p.dim.max(1);
            let kind_value = |ty: &Type, structs: &mut Vec<String>| -> String {
                match ty {
                    Type::Byte => "byte".into(),
                    Type::Int => "int".into(),
                    Type::Float => "float".into(),
                    Type::Bool => "bool".into(),
                    Type::Name => "name".into(),
                    Type::Str => "str".into(),
                    // The class of the object or the metaclass follows the colon (PropertyClass, MetaClass).
                    Type::Object(c) => format!("object:{}", c.rsplit('.').next().unwrap_or(c)),
                    Type::Class(c) => format!("class:{}", c.rsplit('.').next().unwrap_or(c)),
                    Type::Delegate => "delegate".into(),
                    Type::Struct(s) => {
                        if !structs.iter().any(|x| x.eq_ignore_ascii_case(s)) {
                            structs.push(s.clone());
                        }
                        format!("struct:{s}")
                    }
                    Type::Array(_) => "array".into(),
                    Type::Unknown => "?".into(),
                }
            };
            let g = kind_value(&p.ty, structs);
            let mut line = format!("P {} {} {} {} {} {:x}", p.name, g, f.offset, elem, p.dim, p.flags);
            if let Some(b) = f.bit {
                line += &format!(" bit {b:x}");
            }
            if let Type::Array(inner) = &p.ty {
                let (es, _) = self.type_size(inner);
                let gi = kind_value(inner, structs);
                line += &format!(" inner {gi} {es}");
            }
            *out += &line;
            *out += "\n";
        }
    }
}
