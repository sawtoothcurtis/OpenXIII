//! Resolution of HXAudio's external resources.
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub(super) fn resolve(bank: &Path, resource: &str) -> Option<PathBuf> {
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;
    #[link(name="shlwapi")]
    unsafe extern "system" {
        fn PathRemoveFileSpecA(path:*mut c_char)->i32;
        fn PathAddBackslashA(path:*mut c_char)->*mut c_char;
        fn PathAppendA(path:*mut c_char,more:*const c_char)->i32;
        fn PathCanonicalizeA(out:*mut c_char,input:*const c_char)->i32;
        fn PathSkipRootA(path:*const c_char)->*const c_char;
    }
    // The native arrays are MAX_PATH bytes. Never exceed their capacity.
    let original=bank.to_str()?.as_bytes();
    if original.len()>=260 {return None;}
    let mut input=normalize(original);
    input.push(0);
    let more=CString::new(resource).ok()?;
    let mut path=[0u8;260];path[..input.len()].copy_from_slice(&input);
    unsafe {
        if PathRemoveFileSpecA(path.as_mut_ptr().cast())==0 {return None;}
        PathAddBackslashA(path.as_mut_ptr().cast());
        if PathAppendA(path.as_mut_ptr().cast(),more.as_ptr())==0 {return None;}
        let before=path;
        if PathCanonicalizeA(path.as_mut_ptr().cast(),before.as_ptr().cast())==0 {return None;}
        // a substituted DOS drive is expanded, the others are kept.
        let mut drive=[0u8;3];drive[..2].copy_from_slice(&path[..2]);
        let mut device=[0u8;260];
        if query_device(&drive,&mut device)!=0 && device.starts_with(b"\\??\\") {
            let tail=PathSkipRootA(path.as_ptr().cast());
            if tail.is_null() {return None;}
            let prefix=CStr::from_ptr(device.as_ptr().add(4).cast()).to_bytes();
            let suffix=CStr::from_ptr(tail).to_bytes();
            if prefix.len()+suffix.len()+2>=260 {return None;}
            let mut expanded=[0u8;260];expanded[..prefix.len()].copy_from_slice(prefix);
            if PathAppendA(expanded.as_mut_ptr().cast(),tail)==0 {return None;}
            path=expanded;
        }
        let bytes=CStr::from_ptr(path.as_ptr().cast()).to_bytes();
        let mut result=normalize(bytes);
        // The ".." prefix of the bank name is restored after canonicalisation.
        if original.starts_with(b"..") {result.splice(..0,b"..".iter().copied());}
        Some(PathBuf::from(String::from_utf8(result).ok()?))
    }
}

// Replaces / before reducing each run of separators to one.
pub(super) fn normalize(input:&[u8])->Vec<u8> {
    let mut out=Vec::with_capacity(input.len());
    for &b in input {
        let b=if b==b'/' {b'\\'} else {b};
        if b!=b'\\' || out.last()!=Some(&b'\\') {out.push(b);}
    }
    out
}

#[cfg(not(windows))]
pub(super) fn resolve(bank:&Path,resource:&str)->Option<PathBuf> {
    Some(bank.parent()?.join(resource.replace('\\',"/")))
}

#[cfg(windows)]
fn query_device(name:&[u8;3],out:&mut [u8;260])->u32 {
    #[link(name="kernel32")]
    unsafe extern "system" {fn QueryDosDeviceA(name:*const u8,out:*mut u8,max:u32)->u32;}
    unsafe {QueryDosDeviceA(name.as_ptr(),out.as_mut_ptr(),260)}
}

