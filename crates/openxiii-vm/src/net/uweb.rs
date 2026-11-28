//! Natives of UWeb.dll (the server's web administration): UWebRequest (form variables,
//! Base64) and UWebResponse (substitutions and .uhtm pages, binary files).
//!
//! VariableMap (UWebRequest + 0x60) is a TMultiMap<FString, FString> and ReplacementMap a
//! TMap<FString, FString>: their C++ state is kept here, per object. In both, an added
//! key goes to the head of its hash chain, so Find returns the last value
//! added and MultiFind the values from the most recent to the oldest; the
//! keys are compared case-insensitively (FString ==).

use crate::value::Value;
use crate::{ObjId, Vm};

/// Pairs of a UWeb TMap / TMultiMap, in insertion order.
#[derive(Default, Clone)]
pub struct WebMap {
    pairs: Vec<(String, String)>,
}

impl WebMap {
    /// (Find): value of the most recent pair with this key.
    fn find(&self, key: &str) -> Option<&String> {
        self.pairs.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    /// (MultiFind): from the most recent to the oldest.
    fn multi_find(&self, key: &str) -> Vec<String> {
        self.pairs.iter().rev().filter(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone()).collect()
    }

    /// (Add).
    fn add(&mut self, key: &str, value: &str) {
        self.pairs.push((key.to_string(), value.to_string()));
    }

    /// (Set): replaces the value of the pair found by Find, otherwise Add.
    fn set(&mut self, key: &str, value: &str) {
        match self.pairs.iter_mut().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some(p) => p.1 = value.to_string(),
            None => self.add(key, value),
        }
    }
}

/// FString::Caps: ASCII uppercase.
fn caps(s: &str) -> String {
    s.to_ascii_uppercase()
}

/// File name refused by IncludeUHTM / IncludeBinaryFile: starts with "\" or "/", or
/// contains "..\" or "../".
fn dangerous(f: &str) -> bool {
    f.starts_with('\\') || f.starts_with('/') || f.contains("..\\") || f.contains("../")
}

/// UWebRequest::execDecodeBase64: a character outside the alphabet, or "=" at
/// position 0 or 1 of a group, returns an empty string; the result stops at the first null byte.
fn decode_base64(s: &str) -> String {
    const ALPHA: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out: Vec<u8> = vec![0];
    let (mut n, mut i) = (0usize, 0usize);
    let finish = |mut out: Vec<u8>| {
        if let Some(z) = out.iter().position(|&b| b == 0) {
            out.truncate(z);
        }
        out.iter().map(|&b| b as char).collect::<String>()
    };
    for c in s.chars() {
        let need = |out: &mut Vec<u8>, i: usize| {
            while out.len() <= i + 1 {
                out.push(0);
            }
        };
        if c == '=' {
            match n % 4 {
                0 | 1 => return String::new(),
                2 => {
                    i += 1;
                    need(&mut out, i);
                    out[i] = 0;
                }
                _ => {
                    need(&mut out, i);
                    out[i] = 0;
                }
            }
            return finish(out);
        }
        let Some(v) = ALPHA.find(c) else { return String::new() };
        let v = v as u8;
        need(&mut out, i);
        match n % 4 {
            0 => out[i] = v << 2,
            1 => {
                out[i] |= v >> 4;
                i += 1;
                need(&mut out, i);
                out[i] = v << 4;
            }
            2 => {
                out[i] |= v >> 2;
                i += 1;
                need(&mut out, i);
                out[i] = v << 6;
            }
            _ => {
                out[i] |= v;
                i += 1;
            }
        }
        n += 1;
    }
    need_len(&mut out, i);
    out[i] = 0;
    finish(out)
}

fn need_len(out: &mut Vec<u8>, i: usize) {
    while out.len() <= i {
        out.push(0);
    }
}

impl Vm {
    /// UWebResponse::SendInParts: SendText(Text, True), in chunks of 512
    /// characters beyond 512; nothing for an empty string.
    fn web_send_in_parts(&mut self, this: ObjId, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        if len == 0 {
            return;
        }
        let n = len;
        if n <= 0x200 {
            self.event_pub(this, "SendText", vec![Value::Str(text.to_string()), Value::Bool(true)]);
            return;
        }
        let mut at = 0;
        while n - at > 0x200 {
            let part: String = chars[at..at + 0x200].iter().collect();
            self.event_pub(this, "SendText", vec![Value::Str(part), Value::Bool(true)]);
            at += 0x200;
        }
        if at > 0 {
            let rest: String = chars[at..].iter().collect();
            self.event_pub(this, "SendText", vec![Value::Str(rest), Value::Bool(true)]);
        }
    }

    fn web_include_path(&mut self, this: ObjId, file: &str) -> Result<std::path::PathBuf, String> {
        if dangerous(file) {
            return Err(format!("Log: WebServer: Dangerous characters in filename: {file}"));
        }
        let inc = self.prop(this, "IncludePath").to_string();
        if inc.is_empty() {
            return Err(format!("Log: WebServer: Bad IncludePath: {inc}"));
        }
        let dir = self.config.as_ref().map(|c| c.dir().to_path_buf()).unwrap_or_default();
        Ok(dir.join(format!("{inc}\\{file}").replace('\\', "/")))
    }

    pub(crate) fn uweb_native(&mut self, name: &str, this: ObjId, args: &mut [Value]) -> Option<Value> {
        let s = |i: usize| args.get(i).map(|v| v.to_string()).unwrap_or_default();
        Some(match name {
            // UWebRequest::execDecodeBase64.
            "DecodeBase64" if self.is_a(this, "WebRequest") => Value::Str(decode_base64(&s(0))),
            // UWebRequest::execAddVariable: VariableMap.Add(Caps(Name), Value).
            "AddVariable" if self.is_a(this, "WebRequest") => {
                let (k, v) = (caps(&s(0)), s(1));
                self.web_maps.entry(this).or_default().add(&k, &v);
                Value::Void
            }
            // UWebRequest::execGetVariable: Find(Caps(Name)), otherwise DefaultValue.
            "GetVariable" if self.is_a(this, "WebRequest") => {
                let k = caps(&s(0));
                Value::Str(self.web_maps.get(&this).and_then(|m| m.find(&k).cloned()).unwrap_or_else(|| s(1)))
            }
            // UWebRequest::execGetVariableCount: MultiFind(Caps(Name)).Num().
            "GetVariableCount" if self.is_a(this, "WebRequest") => {
                let k = caps(&s(0));
                Value::Int(self.web_maps.get(&this).map_or(0, |m| m.multi_find(&k).len()) as i32)
            }
            // UWebRequest::execGetVariableNumber: MultiFind(Caps(Name))(Number) if
            // Number < Num() (a negative number passes the test, as in the machine code: None here,
            // the original reads before the array), otherwise DefaultValue.
            "GetVariableNumber" if self.is_a(this, "WebRequest") => {
                let k = caps(&s(0));
                let i = args.get(1).map_or(0, |v| v.as_int());
                let all = self.web_maps.get(&this).map(|m| m.multi_find(&k)).unwrap_or_default();
                Value::Str(if i < all.len() as i32 && i >= 0 { all[i as usize].clone() } else { s(2) })
            }
            // UWebResponse::execClearSubst: ReplacementMap emptied.
            "ClearSubst" if self.is_a(this, "WebResponse") => {
                self.web_subst.remove(&this);
                Value::Void
            }
            // UWebResponse::execSubst: emptied if bClear, then Set(Variable, Value).
            "Subst" if self.is_a(this, "WebResponse") => {
                let (k, v) = (s(0), s(1));
                let clear = args.get(2).is_some_and(|v| v.as_bool());
                let m = self.web_subst.entry(this).or_default();
                if clear {
                    *m = WebMap::default();
                }
                m.set(&k, &v);
                Value::Void
            }
            // UWebResponse::execIncludeUHTM: IncludePath\Filename read as text;
            // "%Var%" replaced by ReplacementMap(Var) (empty if missing, "%%" gives "%"); the
            // text goes out through SendInParts, between the variables.
            "IncludeUHTM" if self.is_a(this, "WebResponse") => {
                let file = s(0);
                match self.web_include_path(this, &file) {
                    Err(msg) => self.log.push(msg),
                    Ok(path) => match std::fs::read(&path) {
                        Err(_) => {
                            let inc = self.prop(this, "IncludePath").to_string();
                            self.log.push(format!("Log: WebServer: Unable to open include file {inc}\\{file}"));
                        }
                        Ok(bytes) => {
                            let text: String = bytes.iter().map(|&b| b as char).collect();
                            let mut rest = text.as_str();
                            loop {
                                let Some(a) = rest.find('%') else { break };
                                self.web_send_in_parts(this, &rest[..a]);
                                match rest[a + 1..].find('%') {
                                    // "%" without a closing one: skipped (the following text goes out afterwards).
                                    None => rest = &rest[a + 1..],
                                    Some(b) => {
                                        let var = rest[a + 1..a + 1 + b].to_string();
                                        let value = if var.is_empty() { "%".to_string() } else { self.web_subst.get(&this).and_then(|m| m.find(&var).cloned()).unwrap_or_default() };
                                        self.web_send_in_parts(this, &value);
                                        rest = &rest[a + 1 + b + 1..];
                                    }
                                }
                            }
                            let tail = rest.to_string();
                            self.web_send_in_parts(this, &tail);
                        }
                    },
                }
                Value::Void
            }
            // UWebResponse::execIncludeBinaryFile: the file through SendBinary(Count, B)
            // in blocks of 255 bytes.
            "IncludeBinaryFile" if self.is_a(this, "WebResponse") => {
                let file = s(0);
                match self.web_include_path(this, &file) {
                    Err(msg) => self.log.push(msg),
                    Ok(path) => match std::fs::read(&path) {
                        Err(_) => {
                            let inc = self.prop(this, "IncludePath").to_string();
                            self.log.push(format!("Log: WebServer: Unable to open include file {inc}\\{file}"));
                        }
                        Ok(bytes) => {
                            for chunk in bytes.chunks(0xff) {
                                let mut b: Vec<Value> = chunk.iter().map(|&x| Value::Byte(x)).collect();
                                b.resize(0xff, Value::Byte(0));
                                self.event_pub(this, "SendBinary", vec![Value::Int(chunk.len() as i32), Value::Array(b)]);
                            }
                        }
                    },
                }
                Value::Void
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64() {
        assert_eq!(decode_base64("YWRtaW46c2VjcmV0"), "admin:secret");
        assert_eq!(decode_base64("YQ=="), "a");
        assert_eq!(decode_base64("YWI="), "ab");
        assert_eq!(decode_base64("Y*=="), "");
        assert_eq!(decode_base64("=AAA"), "");
    }

    #[test]
    fn multimap() {
        let mut m = WebMap::default();
        m.add("A", "1");
        m.add("A", "2");
        assert_eq!(m.find("a"), Some(&"2".to_string()));
        assert_eq!(m.multi_find("A"), vec!["2".to_string(), "1".to_string()]);
        m.set("A", "3");
        assert_eq!(m.multi_find("A"), vec!["3".to_string(), "1".to_string()]);
    }
}
