//! Core's appInit: what it decides at startup that has an effect on the game. The
//! plumbing calls (allocator, log, memory stack, FName / UObject::StaticInit) are not reimplemented.

use std::collections::HashMap;

use crate::value::{app_strfind, parse_fstring};

/// Core's GCRCTable: CRC32 with polynomial 0x04C11DB7, filled in by appInit.
pub fn table_crc() -> [u32; 256] {
    let mut t = [0u32; 256];
    for (i, e) in t.iter_mut().enumerate() {
        let mut v = (i as u32) << 24;
        for _ in 0..8 {
            v = if (v as i32) < 0 { v.wrapping_mul(2) ^ 0x04C1_1DB7 } else { v << 1 };
        }
        *e = v;
    }
    t
}

/// Core's appCeilLogTwo table (257 bytes), filled in by appInit.
pub fn table_ceil_log_two() -> [u8; 257] {
    let mut t = [0u8; 257];
    let (mut b, mut n) = (0xFFu8, 0i32);
    for (i, e) in t.iter_mut().enumerate() {
        *e = b.wrapping_add(1);
        let rises = if i == 0 {
            true
        } else {
            n += 1;
            1i32.wrapping_shl((b & 0x1F) as u32) <= n
        };
        if rises {
            n = 0;
            b = b.wrapping_add(1);
        }
    }
    t
}

/// ParseParam: the parameter is searched for by appStrfind starting from the character
/// after the start of the last match, and counts when "-" or "/" precedes it; what
/// follows it is not looked at ("-STRICTX" is "STRICT").
pub fn parse_param(stream: &str, param: &str) -> bool {
    if stream.is_empty() {
        return false;
    }
    let mut start_value = 0usize;
    loop {
        let from_time = start_value + 1;
        let Some(off) = app_strfind(&stream[from_time.min(stream.len())..], param) else { return false };
        let p = from_time + off;
        let c = stream.as_bytes()[p - 1];
        if c == b'-' || c == b'/' {
            return true;
        }
        start_value = p;
    }
}

/// Command line without the program name (appInit: name in quotes or up to the first
/// space, then one more character; quote never closed: everything is skipped).
pub fn line_command(cmd: &str) -> &str {
    let b = cmd.as_bytes();
    let c = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut i = 0usize;
    if c(0) == b'"' {
        loop {
            i += 1;
            if c(i) == 0 {
                return &cmd[i..];
            }
            if c(i) == b'"' {
                break;
            }
        }
        loop {
            i += 1;
            if c(i) == 0 {
                return &cmd[i..];
            }
            if c(i) == b' ' {
                break;
            }
        }
    } else {
        loop {
            if c(i) == 0 {
                return &cmd[i..];
            }
            if c(i) == b' ' {
                break;
            }
            i += 1;
        }
    }
    &cmd[i + 1..]
}

/// Name of a file as a test's dummy disk sees it: last component, lowercase.
pub fn name_file(path_buf: &str) -> String {
    path_buf.replace('\\', "/").rsplit('/').next().unwrap_or("").to_lowercase()
}

#[derive(Debug, Default, PartialEq)]
pub struct AppInit {
    /// The game's ini ("System") and the player's ini ("User"), as given to GConfig.
    pub ini: String,
    pub user_ini: String,
    pub line_value: String,
    pub strict: bool,
    /// Files created (name, content) when `create` is true and the ini is missing.
    pub created: Vec<(String, Vec<u8>)>,
    /// GError calls (default ini not found).
    pub errors_list: usize,
    /// UObject::SetLanguage: [Engine.Engine] Language of the game's ini, if it exists.
    pub language_value: Option<String>,
}

/// appInit(base, cmd, …, create, …). `files_list`: the System folder (lowercase name → content).
/// `language_value` reads [Engine.Engine] Language in the game's ini once the files are created.
pub fn app_init(base: &str, cmd: &str, create: bool, files_list: &HashMap<String, Vec<u8>>, language_value: &dyn Fn(&str, &HashMap<String, Vec<u8>>) -> Option<String>) -> AppInit {
    let line_value = line_command(cmd).to_string();
    let strict = parse_param(&line_value, "STRICT");
    let such = |v: Option<String>, default_value: String| v.map(|s| s.chars().take(255).collect::<String>()).unwrap_or(default_value);
    let ini = such(parse_fstring(&line_value, "INI="), format!("{base}.ini"));
    let mut files_list = files_list.clone();
    let mut created = Vec::new();
    let mut errors_list = 0;
    let mut creates = |name_value: &str, model: &str, files_list: &mut HashMap<String, Vec<u8>>| {
        if !create || files_list.contains_key(&name_file(name_value)) {
            return;
        }
        match files_list.get(&model.to_lowercase()).cloned() {
            Some(c) => {
                if !c.is_empty() {
                    files_list.insert(name_file(name_value), c.clone());
                    created.push((name_file(name_value), c));
                }
            }
            None => errors_list += 1,
        }
    };
    creates(&ini, "Default.ini", &mut files_list);
    let user_ini = such(parse_fstring(&line_value, "USERINI="), "User.ini".to_string());
    creates(&user_ini, "DefUser.ini", &mut files_list);
    let language_value = language_value(&ini, &files_list);
    AppInit { ini, user_ini, line_value, strict, created, errors_list, language_value }
}

