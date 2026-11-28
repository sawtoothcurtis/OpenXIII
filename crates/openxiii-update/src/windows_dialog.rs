//! Windows: the question in a task dialog with three named buttons (the system's own window). The program has no manifest asking for the version 6 common controls, which hold
//! the task dialog: they are taken for this window only (activation context), and when they cannot be,
//! a plain message box asks instead.

use windows_sys::Win32::Foundation::{BOOL, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::ApplicationInstallationAndServicing::{ActivateActCtx, CreateActCtxW, DeactivateActCtx, ReleaseActCtx, ACTCTXW};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::Controls::{
    TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW, TD_INFORMATION_ICON,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONINFORMATION, MB_SETFOREGROUND, MB_YESNO};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// Asks for version 6 of the common controls (needed for the task dialog).
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency><dependentAssembly>
    <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
  </dependentAssembly></dependency>
</assembly>
"#;

type TaskDialogIndirect = unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut BOOL) -> i32;

/// Shows a task dialog with `buttons` (id, label), the first one by default. None when the task dialog
/// is not available.
fn task_dialog(title: &str, heading: &str, text: &str, buttons: &[(i32, &str)]) -> Option<i32> {
    let manifest = std::env::temp_dir().join("OpenXIII-dialog.manifest");
    std::fs::write(&manifest, MANIFEST).ok()?;
    let source = wide(&manifest.to_string_lossy());
    let ctx_info = ACTCTXW { cbSize: std::mem::size_of::<ACTCTXW>() as u32, lpSource: source.as_ptr(), ..unsafe { std::mem::zeroed() } };
    unsafe {
        let ctx: HANDLE = CreateActCtxW(&ctx_info);
        let _ = std::fs::remove_file(&manifest);
        if ctx == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut cookie = 0usize;
        let result = (|| {
            if ActivateActCtx(ctx, &mut cookie) == 0 {
                return None;
            }
            let lib = LoadLibraryW(wide("comctl32.dll").as_ptr());
            if lib.is_null() {
                return None;
            }
            let f = GetProcAddress(lib, c"TaskDialogIndirect".as_ptr().cast())?;
            let f: TaskDialogIndirect = std::mem::transmute(f);
            let labels: Vec<Vec<u16>> = buttons.iter().map(|(_, l)| wide(l)).collect();
            let list: Vec<TASKDIALOG_BUTTON> =
                buttons.iter().zip(&labels).map(|((id, _), l)| TASKDIALOG_BUTTON { nButtonID: *id, pszButtonText: l.as_ptr() }).collect();
            let (title, heading, text) = (wide(title), wide(heading), wide(text));
            let mut config: TASKDIALOGCONFIG = std::mem::zeroed();
            config.cbSize = std::mem::size_of::<TASKDIALOGCONFIG>() as u32;
            config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW;
            config.pszWindowTitle = title.as_ptr();
            config.Anonymous1.pszMainIcon = TD_INFORMATION_ICON;
            config.pszMainInstruction = heading.as_ptr();
            config.pszContent = text.as_ptr();
            config.cButtons = list.len() as u32;
            config.pButtons = list.as_ptr();
            config.nDefaultButton = buttons.first().map_or(0, |b| b.0);
            let mut pressed = 0;
            (f(&config, &mut pressed, std::ptr::null_mut(), std::ptr::null_mut()) >= 0).then_some(pressed)
        })();
        if cookie != 0 {
            DeactivateActCtx(0, cookie);
        }
        ReleaseActCtx(ctx);
        result
    }
}

fn message_box(title: &str, text: &str, flags: u32) -> i32 {
    unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide(title).as_ptr(), flags | MB_SETFOREGROUND) }
}

/// The three answers, Yes/No message box when the task dialog is not available (No is "later").
pub fn ask(title: &str, heading: &str, text: &str, open: &str, later: &str, never: &str) -> super::Answer {
    use super::Answer;
    const OPEN: i32 = 1001;
    const LATER: i32 = 1002;
    const NEVER: i32 = 1003;
    match task_dialog(title, heading, text, &[(OPEN, open), (LATER, later), (NEVER, never)]) {
        Some(OPEN) => Answer::Open,
        Some(NEVER) => Answer::Never,
        Some(_) => Answer::Later,
        None => {
            if message_box(title, &format!("{heading}\n\n{text}\n\n{open}?"), MB_YESNO | MB_ICONINFORMATION) == IDYES {
                Answer::Open
            } else {
                Answer::Later
            }
        }
    }
}
