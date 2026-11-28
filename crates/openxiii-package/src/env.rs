//! Environment variables of OpenXIII (`OXIII_*`), read through [`var`] and [`var_os`].

use std::ffi::{OsStr, OsString};

/// Like `std::env::var`.
pub fn var<K: AsRef<OsStr>>(key: K) -> Result<String, std::env::VarError> {
    std::env::var(key)
}

/// Like `std::env::var_os`.
pub fn var_os<K: AsRef<OsStr>>(key: K) -> Option<OsString> {
    std::env::var_os(key)
}
