//! Optional desktop shortcut offered by the launcher when the player points to the original game.
//!
//! The shortcut's icon is the XIII logo cut out of the original game's Help\XIIILogo.bmp: the
//! .ico is built on the player's computer, in OpenXIII's data folder (never shipped). The game
//! executable, its window and the taskbar keep OpenXIII's own icon. If the picture is missing or
//! too small, the shortcut uses the executable's icon.

use std::path::{Path, PathBuf};

/// Area of the logo in XIIILogo.bmp (640x448, top-left origin): square centred on the "XIII"
/// letters (x 223..385, y 68..261).
const LOGO_X: usize = 204;
const LOGO_Y: usize = 63;
const LOGO_SIZE: usize = 200;
/// Sizes of the images stored in the .ico.
const ICON_SIZES: [usize; 4] = [256, 48, 32, 16];

/// RGBA picture, rows from top to bottom.
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// Entry of `dir` named `name`, ignoring case.
fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(name)))
}

/// <game>\Help\XIIILogo.bmp, ignoring case.
pub fn logo_bmp(game: &Path) -> Option<PathBuf> {
    find_ci(&find_ci(game, "Help")?, "XIIILogo.bmp").filter(|p| p.is_file())
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// Uncompressed Windows BMP (8, 24 or 32 bits per pixel; bottom-up or top-down) to RGBA.
pub fn decode_bmp(b: &[u8]) -> Option<Rgba> {
    if b.get(0..2)? != b"BM" {
        return None;
    }
    let data_offset = u32_at(b, 10)? as usize;
    let header_size = u32_at(b, 14)? as usize;
    if header_size < 40 {
        return None;
    }
    let width = u32_at(b, 18)? as i32;
    let raw_height = u32_at(b, 22)? as i32;
    let bpp = u16_at(b, 28)?;
    let compression = u32_at(b, 30)?;
    if width <= 0 || raw_height == 0 || !(compression == 0 || (compression == 3 && bpp == 32)) {
        return None;
    }
    let (width, height) = (width as usize, raw_height.unsigned_abs() as usize);
    let bottom_up = raw_height > 0;
    let palette: Vec<[u8; 3]> = if bpp == 8 {
        let used = u32_at(b, 46)? as usize;
        let count = if used == 0 { 256 } else { used.min(256) };
        let start = 14 + header_size;
        (0..count).map(|i| b.get(start + 4 * i..start + 4 * i + 3).map(|c| [c[2], c[1], c[0]])).collect::<Option<_>>()?
    } else {
        Vec::new()
    };
    let stride = (width * bpp as usize).div_ceil(32) * 4;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let row = if bottom_up { height - 1 - y } else { y };
        let line = b.get(data_offset + row * stride..data_offset + row * stride + stride)?;
        for x in 0..width {
            let rgb = match bpp {
                8 => *palette.get(line[x] as usize)?,
                24 => [line[3 * x + 2], line[3 * x + 1], line[3 * x]],
                32 => [line[4 * x + 2], line[4 * x + 1], line[4 * x]],
                _ => return None,
            };
            pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    Some(Rgba { width, height, pixels })
}

/// `size`x`size` square of `src` at (`x`, `y`), or None if it does not fit.
pub fn crop(src: &Rgba, x: usize, y: usize, size: usize) -> Option<Rgba> {
    if x + size > src.width || y + size > src.height {
        return None;
    }
    let mut pixels = Vec::with_capacity(size * size * 4);
    for row in y..y + size {
        let start = (row * src.width + x) * 4;
        pixels.extend_from_slice(&src.pixels[start..start + size * 4]);
    }
    Some(Rgba { width: size, height: size, pixels })
}

/// Square resize: area average when shrinking, bilinear when enlarging.
pub fn resize(src: &Rgba, size: usize) -> Rgba {
    let scale = src.width as f32 / size as f32;
    let mut pixels = Vec::with_capacity(size * size * 4);
    for dy in 0..size {
        for dx in 0..size {
            let mut acc = [0f32; 4];
            if scale > 1.0 {
                let (x0, x1) = ((dx as f32 * scale) as usize, (((dx + 1) as f32 * scale).ceil() as usize).min(src.width));
                let (y0, y1) = ((dy as f32 * scale) as usize, (((dy + 1) as f32 * scale).ceil() as usize).min(src.height));
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let p = &src.pixels[(sy * src.width + sx) * 4..][..4];
                        for c in 0..4 {
                            acc[c] += p[c] as f32;
                        }
                    }
                }
                let n = ((x1 - x0) * (y1 - y0)) as f32;
                acc.iter_mut().for_each(|a| *a /= n);
            } else {
                let fx = ((dx as f32 + 0.5) * scale - 0.5).clamp(0.0, (src.width - 1) as f32);
                let fy = ((dy as f32 + 0.5) * scale - 0.5).clamp(0.0, (src.height - 1) as f32);
                let (x0, y0) = (fx as usize, fy as usize);
                let (x1, y1) = ((x0 + 1).min(src.width - 1), (y0 + 1).min(src.height - 1));
                let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                let at = |x: usize, y: usize, c: usize| src.pixels[(y * src.width + x) * 4 + c] as f32;
                for c in 0..4 {
                    let top = at(x0, y0, c) * (1.0 - tx) + at(x1, y0, c) * tx;
                    let bottom = at(x0, y1, c) * (1.0 - tx) + at(x1, y1, c) * tx;
                    acc[c] = top * (1.0 - ty) + bottom * ty;
                }
            }
            pixels.extend(acc.iter().map(|a| a.round().clamp(0.0, 255.0) as u8));
        }
    }
    Rgba { width: size, height: size, pixels }
}

pub fn encode_png(img: &Rgba) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, img.width as u32, img.height as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().ok()?.write_image_data(&img.pixels).ok()?;
    Some(out)
}

/// .ico with one PNG image per size of ICON_SIZES.
pub fn build_ico(logo: &Rgba) -> Option<Vec<u8>> {
    let images: Vec<Vec<u8>> = ICON_SIZES.iter().map(|&s| encode_png(&resize(logo, s))).collect::<Option<_>>()?;
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (&size, data) in ICON_SIZES.iter().zip(&images) {
        let dim = if size >= 256 { 0 } else { size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes()); // planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    images.iter().for_each(|d| ico.extend_from_slice(d));
    Some(ico)
}

/// The logo cut out of the game's XIIILogo.bmp (None if the file is missing or too small).
pub fn logo_from_game(game: &Path) -> Option<Rgba> {
    let bmp = decode_bmp(&std::fs::read(logo_bmp(game)?).ok()?)?;
    crop(&bmp, LOGO_X, LOGO_Y, LOGO_SIZE)
}

/// The main menu's logo texture (XIIIMenu.Menu.MprincipalLogo, 256x256), read from the game's
/// packages.
pub fn logo_from_menu_texture(game: &Path) -> Option<Rgba> {
    let mut library = openxiii_package::library::Library::new(game).ok()?;
    let (loaded, index) = library.resolve_class("XIIIMenu.Menu.MprincipalLogo", "Texture")?;
    let texture = openxiii_assets::texture::Texture::read(&loaded.package, &loaded.data, index).ok()?;
    let (width, height, pixels) = texture.decode(0).ok()?;
    let mut logo = Rgba { width: width as usize, height: height as usize, pixels };
    // The picture is opaque: an alpha channel left at zero by the texture format is filled in.
    logo.pixels.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    Some(square(&logo))
}

/// `img` centred on a black square (unchanged if already square).
pub fn square(img: &Rgba) -> Rgba {
    if img.width == img.height {
        return Rgba { width: img.width, height: img.height, pixels: img.pixels.clone() };
    }
    let size = img.width.max(img.height);
    let (ox, oy) = ((size - img.width) / 2, (size - img.height) / 2);
    let mut pixels: Vec<u8> = [0, 0, 0, 255].repeat(size * size);
    for y in 0..img.height {
        let dst = ((y + oy) * size + ox) * 4;
        pixels[dst..dst + img.width * 4].copy_from_slice(&img.pixels[y * img.width * 4..(y + 1) * img.width * 4]);
    }
    Rgba { width: size, height: size, pixels }
}

/// Pictures of the game the shortcut's logo can come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogoSource {
    /// The main menu's logo texture (XIIIMenu.Menu.MprincipalLogo).
    MenuTexture = 0,
    /// The logo cut out of Help\XIIILogo.bmp.
    HelpBitmap = 1,
}

/// Logo of the desktop shortcut; the other picture is used when it is missing.
pub const DESKTOP_LOGO: LogoSource = LogoSource::MenuTexture;
/// Trial builds only: one desktop shortcut per picture ("XIII (aide)" and "XIII (menu)"), to
/// compare them.
pub const DESKTOP_COMPARE: bool = false;

impl LogoSource {
    pub fn other(self) -> Self {
        match self {
            LogoSource::MenuTexture => LogoSource::HelpBitmap,
            LogoSource::HelpBitmap => LogoSource::MenuTexture,
        }
    }
}

/// The square logo of `source` from the game in `game`.
pub fn logo(game: &Path, source: LogoSource) -> Option<Rgba> {
    match source {
        LogoSource::MenuTexture => logo_from_menu_texture(game),
        LogoSource::HelpBitmap => logo_from_game(game),
    }
}

/// The logo of `source`, or with `fallback` the other picture when it is missing; with the
/// source actually used.
pub fn logo_with_fallback(game: &Path, source: LogoSource, fallback: bool) -> Option<(LogoSource, Rgba)> {
    let order: &[LogoSource] = if fallback { &[source, source.other()] } else { &[source] };
    order.iter().find_map(|&s| logo(game, s).map(|img| (s, img)))
}

/// Writes the logo .ico (`name`) into `data_dir`; its path, or None (no picture, or writing failed).
pub fn write_logo_icon(game: &Path, data_dir: &Path, source: LogoSource, fallback: bool, name: &str) -> Option<PathBuf> {
    let ico = build_ico(&logo_with_fallback(game, source, fallback)?.1)?;
    std::fs::create_dir_all(data_dir).ok()?;
    let path = data_dir.join(name);
    std::fs::write(&path, ico).ok()?;
    Some(path)
}

/// Creates `<desktop>\XIII.lnk` to this executable, with the logo icon when one of the game's
/// pictures is usable (otherwise the executable's icon). Failures are silent: the shortcut is only
/// a convenience.
#[cfg(windows)]
pub fn create_on_desktop(game: &Path, data_dir: &Path) {
    let (Some(desktop), Ok(exe)) = (win::desktop_dir(), std::env::current_exe()) else { return };
    if DESKTOP_COMPARE {
        for (source, label, ico) in [(LogoSource::HelpBitmap, "XIII (aide)", "XIII-aide.ico"), (LogoSource::MenuTexture, "XIII (menu)", "XIII-menu.ico")] {
            let icon = write_logo_icon(game, data_dir, source, false, ico);
            let _ = win::create_shortcut(&desktop.join(format!("{label}.lnk")), &exe, icon.as_deref());
        }
        return;
    }
    let icon = write_logo_icon(game, data_dir, DESKTOP_LOGO, true, "XIII.ico");
    let _ = win::create_shortcut(&desktop.join("XIII.lnk"), &exe, icon.as_deref());
}

#[cfg(windows)]
pub use win::{ask_with_checkbox, create_shortcut, with_progress};

/// Runs `work` (which calls its argument once per step done, with the next step's text) while a
/// window shows its progress over `steps` steps; returns when it is finished. Without that
/// window (another system), `work` simply runs.
#[cfg(not(windows))]
pub fn with_progress(_title: &str, _text: &str, _steps: u32, work: impl FnOnce(&dyn Fn(&str)) + Send) {
    work(&|_: &str| {});
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    type HRESULT = i32;

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);

    const CLSID_SHELL_LINK: Guid = Guid(0x00021401, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_ISHELL_LINK_W: Guid = Guid(0x000214F9, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_IPERSIST_FILE: Guid = Guid(0x0000010B, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const FOLDERID_DESKTOP: Guid = Guid(0xB4BFCC3A, 0xDB2C, 0x424C, [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41]);
    const CLSCTX_INPROC_SERVER: u32 = 1;
    const COINIT_APARTMENTTHREADED: u32 = 2;

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> HRESULT;
        fn CoUninitialize();
        fn CoCreateInstance(clsid: *const Guid, outer: *mut c_void, ctx: u32, iid: *const Guid, out: *mut *mut c_void) -> HRESULT;
        fn CoTaskMemFree(p: *mut c_void);
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHGetKnownFolderPath(id: *const Guid, flags: u32, token: *mut c_void, out: *mut *mut u16) -> HRESULT;
    }

    /// The first methods of IShellLinkW, up to SetPath (vtable order of ShObjIdl.h).
    #[repr(C)]
    struct ShellLinkVtbl {
        query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_path: usize,
        get_id_list: usize,
        set_id_list: usize,
        get_description: usize,
        set_description: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
        get_working_directory: usize,
        set_working_directory: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
        get_arguments: usize,
        set_arguments: usize,
        get_hotkey: usize,
        set_hotkey: usize,
        get_show_cmd: usize,
        set_show_cmd: usize,
        get_icon_location: usize,
        set_icon_location: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
        set_relative_path: usize,
        resolve: usize,
        set_path: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    }

    /// IPersistFile up to Save.
    #[repr(C)]
    struct PersistFileVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_class_id: usize,
        is_dirty: usize,
        load: usize,
        save: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
    }

    fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain([0]).collect()
    }

    fn check(hr: HRESULT) -> Result<(), HRESULT> {
        if hr < 0 { Err(hr) } else { Ok(()) }
    }

    /// The user's desktop folder (follows redirection, e.g. to a synced folder).
    pub fn desktop_dir() -> Option<PathBuf> {
        let mut p: *mut u16 = std::ptr::null_mut();
        let hr = unsafe { SHGetKnownFolderPath(&FOLDERID_DESKTOP, 0, std::ptr::null_mut(), &mut p) };
        if p.is_null() {
            return None;
        }
        let path = (hr >= 0).then(|| {
            let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
            PathBuf::from(String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) }))
        });
        unsafe { CoTaskMemFree(p.cast()) };
        path
    }

    /// Shell link `lnk` to `target` (working directory: its folder), with `icon` (an .ico file)
    /// or the target's own icon.
    pub fn create_shortcut(lnk: &Path, target: &Path, icon: Option<&Path>) -> Result<(), HRESULT> {
        let init = unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
        let result = unsafe { write_link(lnk, target, icon) };
        if init >= 0 {
            unsafe { CoUninitialize() };
        }
        result
    }

    unsafe fn write_link(lnk: &Path, target: &Path, icon: Option<&Path>) -> Result<(), HRESULT> {
        let mut link: *mut c_void = std::ptr::null_mut();
        check(CoCreateInstance(&CLSID_SHELL_LINK, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELL_LINK_W, &mut link))?;
        let vt = &**(link as *mut *const ShellLinkVtbl);
        let result = (|| {
            check((vt.set_path)(link, wide(target).as_ptr()))?;
            if let Some(dir) = target.parent() {
                check((vt.set_working_directory)(link, wide(dir).as_ptr()))?;
            }
            check((vt.set_description)(link, wide("OpenXIII").as_ptr()))?;
            let (icon_file, index) = icon.map_or((target, 0), |i| (i, 0));
            check((vt.set_icon_location)(link, wide(icon_file).as_ptr(), index))?;
            let mut file: *mut c_void = std::ptr::null_mut();
            check((vt.query_interface)(link, &IID_IPERSIST_FILE, &mut file))?;
            let pf = &**(file as *mut *const PersistFileVtbl);
            let saved = check((pf.save)(file, wide(lnk).as_ptr(), 1));
            (pf.release)(file);
            saved
        })();
        (vt.release)(link);
        result
    }

    // Task dialog with a check box (comctl32 version 6, reached through an activation context
    // built from the manifest resource 2 embedded by build.rs; the process itself keeps no manifest).

    #[repr(C)]
    struct ActCtxW {
        size: u32,
        flags: u32,
        source: *const u16,
        processor_architecture: u16,
        lang_id: u16,
        assembly_directory: *const u16,
        resource_name: *const u16,
        application_name: *const u16,
        module: *mut c_void,
    }

    const ACTCTX_FLAG_RESOURCE_NAME_VALID: u32 = 0x8;
    const ACTCTX_FLAG_HMODULE_VALID: u32 = 0x80;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn CreateActCtxW(ctx: *const ActCtxW) -> isize;
        fn ActivateActCtx(ctx: isize, cookie: *mut usize) -> i32;
        fn DeactivateActCtx(flags: u32, cookie: usize) -> i32;
        fn ReleaseActCtx(ctx: isize);
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }

    #[repr(C, packed(1))]
    struct TaskDialogButton {
        id: i32,
        text: *const u16,
    }

    #[repr(C, packed(1))]
    struct TaskDialogConfig {
        size: u32,
        parent: *mut c_void,
        instance: *mut c_void,
        flags: u32,
        common_buttons: u32,
        window_title: *const u16,
        main_icon: *const u16,
        main_instruction: *const u16,
        content: *const u16,
        button_count: u32,
        buttons: *const TaskDialogButton,
        default_button: i32,
        radio_button_count: u32,
        radio_buttons: *const TaskDialogButton,
        default_radio_button: i32,
        verification_text: *const u16,
        expanded_information: *const u16,
        expanded_control_text: *const u16,
        collapsed_control_text: *const u16,
        footer_icon: *const u16,
        footer: *const u16,
        callback: *const c_void,
        callback_data: isize,
        width: u32,
    }

    type TaskDialogIndirectFn = unsafe extern "system" fn(*const TaskDialogConfig, *mut i32, *mut i32, *mut i32) -> HRESULT;

    const TDCBF_OK_BUTTON: u32 = 0x1;
    const TDCBF_CANCEL_BUTTON: u32 = 0x8;
    const IDOK: i32 = 1;
    const TD_INFORMATION_ICON: usize = 0xFFFD;

    /// OK/Cancel dialog with a check box (unchecked at first): Some((ok, checked)), or None when the
    /// task dialog is not available.
    pub fn ask_with_checkbox(title: &str, text: &str, checkbox: &str) -> Option<(bool, bool)> {
        with_comctl6(|| unsafe { show(title, text, checkbox) })
    }

    /// `f` with comctl32 version 6 active (None when it cannot be).
    fn with_comctl6<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let ctx = ActCtxW {
                size: std::mem::size_of::<ActCtxW>() as u32,
                flags: ACTCTX_FLAG_RESOURCE_NAME_VALID | ACTCTX_FLAG_HMODULE_VALID,
                source: std::ptr::null(),
                processor_architecture: 0,
                lang_id: 0,
                assembly_directory: std::ptr::null(),
                resource_name: 2 as *const u16,
                application_name: std::ptr::null(),
                module,
            };
            let handle = CreateActCtxW(&ctx);
            if handle == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut cookie = 0usize;
            let answer = if ActivateActCtx(handle, &mut cookie) != 0 {
                let answer = f();
                DeactivateActCtx(0, cookie);
                answer
            } else {
                None
            };
            ReleaseActCtx(handle);
            answer
        }
    }

    fn task_dialog_fn() -> Option<TaskDialogIndirectFn> {
        unsafe {
            let comctl = LoadLibraryW(wide("comctl32.dll").as_ptr());
            if comctl.is_null() {
                return None;
            }
            let proc = GetProcAddress(comctl, b"TaskDialogIndirect\0".as_ptr());
            (!proc.is_null()).then(|| std::mem::transmute::<*mut c_void, TaskDialogIndirectFn>(proc))
        }
    }

    #[link(name = "user32")]
    extern "system" {
        fn SendMessageW(window: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
    }

    const TDF_SHOW_PROGRESS_BAR: u32 = 0x0200;
    const TDF_CALLBACK_TIMER: u32 = 0x0800;
    const TDN_CREATED: u32 = 0;
    const TDN_BUTTON_CLICKED: u32 = 2;
    const TDN_TIMER: u32 = 4;
    const TDM_CLICK_BUTTON: u32 = 0x0400 + 102;
    const TDM_SET_PROGRESS_BAR_POS: u32 = 0x0400 + 106;
    const TDM_SET_ELEMENT_TEXT: u32 = 0x0400 + 108;
    const TDM_ENABLE_BUTTON: u32 = 0x0400 + 111;
    const TDE_CONTENT: usize = 0;
    const S_FALSE: HRESULT = 1;

    /// What the work thread tells the window.
    struct Progress {
        steps: u32,
        done: std::sync::atomic::AtomicU32,
        finished: std::sync::atomic::AtomicBool,
        /// The current step's text (shown when `shown` differs from `version`).
        text: std::sync::Mutex<(u32, Vec<u16>)>,
        shown: std::sync::atomic::AtomicU32,
        /// The text being shown (kept alive while the window shows it).
        showing: std::sync::Mutex<Vec<u16>>,
    }

    unsafe extern "system" fn progress_callback(window: *mut c_void, msg: u32, _wparam: usize, _lparam: isize, data: isize) -> HRESULT {
        use std::sync::atomic::Ordering::{Acquire, Relaxed};
        let p = &*(data as *const Progress);
        match msg {
            TDN_CREATED => {
                SendMessageW(window, TDM_ENABLE_BUTTON, IDOK as usize, 0);
            }
            TDN_TIMER => {
                let done = p.done.load(Acquire).min(p.steps);
                SendMessageW(window, TDM_SET_PROGRESS_BAR_POS, (done * 100 / p.steps.max(1)) as usize, 0);
                if let (Ok(text), Ok(mut showing)) = (p.text.lock(), p.showing.lock()) {
                    if text.0 != p.shown.load(Relaxed) {
                        p.shown.store(text.0, Relaxed);
                        *showing = text.1.clone();
                        SendMessageW(window, TDM_SET_ELEMENT_TEXT, TDE_CONTENT, showing.as_ptr() as isize);
                    }
                }
                if p.finished.load(Acquire) {
                    SendMessageW(window, TDM_CLICK_BUTTON, IDOK as usize, 0);
                }
            }
            TDN_BUTTON_CLICKED if !p.finished.load(Acquire) => return S_FALSE,
            _ => {}
        }
        0
    }

    /// See the version for other systems.
    pub fn with_progress(title: &str, text: &str, steps: u32, work: impl FnOnce(&dyn Fn(&str)) + Send) {
        use std::sync::atomic::Ordering::{AcqRel, Release};
        let progress = Progress {
            steps,
            done: 0.into(),
            finished: false.into(),
            text: std::sync::Mutex::new((0, wide(text))),
            shown: 0.into(),
            showing: std::sync::Mutex::new(Vec::new()),
        };
        let p = &progress;
        std::thread::scope(|scope| {
            let worker = scope.spawn(move || {
                work(&|next: &str| {
                    p.done.fetch_add(1, AcqRel);
                    if next.is_empty() {
                        return;
                    }
                    if let Ok(mut t) = p.text.lock() {
                        *t = (t.0 + 1, wide(format!("{}\n\n{next}", text_of(&t.1))));
                    }
                });
                p.finished.store(true, Release);
            });
            let shown = with_comctl6(|| unsafe {
                let task_dialog = task_dialog_fn()?;
                let title = wide(title);
                let first = wide(text);
                let mut config: TaskDialogConfig = std::mem::zeroed();
                config.size = std::mem::size_of::<TaskDialogConfig>() as u32;
                config.flags = TDF_SHOW_PROGRESS_BAR | TDF_CALLBACK_TIMER;
                config.common_buttons = TDCBF_OK_BUTTON;
                config.window_title = title.as_ptr();
                config.main_icon = TD_INFORMATION_ICON as *const u16;
                config.content = first.as_ptr();
                config.callback = progress_callback as *const c_void;
                config.callback_data = p as *const Progress as isize;
                check(task_dialog(&config, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())).ok()
            });
            if shown.is_none() {
                // No window: the work goes on alone.
            }
            let _ = worker.join();
        });
    }

    /// The first paragraph of a wide text (the text given at the start).
    fn text_of(w: &[u16]) -> String {
        let s = String::from_utf16_lossy(w.strip_suffix(&[0]).unwrap_or(w));
        s.split("\n\n").next().unwrap_or("").to_string()
    }

    unsafe fn show(title: &str, text: &str, checkbox: &str) -> Option<(bool, bool)> {
        let task_dialog = task_dialog_fn()?;
        let (title, text, checkbox) = (wide(title), wide(text), wide(checkbox));
        let mut config: TaskDialogConfig = std::mem::zeroed();
        config.size = std::mem::size_of::<TaskDialogConfig>() as u32;
        // No flag: the check box starts unchecked (the player opts in).
        config.flags = 0;
        config.common_buttons = TDCBF_OK_BUTTON | TDCBF_CANCEL_BUTTON;
        config.window_title = title.as_ptr();
        config.main_icon = TD_INFORMATION_ICON as *const u16;
        config.content = text.as_ptr();
        config.default_button = IDOK;
        config.verification_text = checkbox.as_ptr();
        let (mut button, mut checked) = (0i32, 0i32);
        check(task_dialog(&config, &mut button, std::ptr::null_mut(), &mut checked)).ok()?;
        Some((button == IDOK, checked != 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 8-bit bottom-up BMP of `w`x`h` whose pixel (x, y) (top-left origin) has palette index
    /// (x + 3 * y) % 256, palette entry i = (r: i, g: 255 - i, b: i / 2).
    fn bmp_8bit(w: usize, h: usize) -> Vec<u8> {
        let stride = w.div_ceil(4) * 4;
        let data_offset = 14 + 40 + 256 * 4;
        let mut b = Vec::new();
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&((data_offset + stride * h) as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&(data_offset as u32).to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&(w as i32).to_le_bytes());
        b.extend_from_slice(&(h as i32).to_le_bytes()); // positive: bottom-up
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&8u16.to_le_bytes());
        b.extend_from_slice(&[0u8; 16]); // compression, image size, resolution
        b.extend_from_slice(&256u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        for i in 0..256u32 {
            b.extend_from_slice(&[(i / 2) as u8, (255 - i) as u8, i as u8, 0]);
        }
        for row in (0..h).rev() {
            let mut line: Vec<u8> = (0..w).map(|x| ((x + 3 * row) % 256) as u8).collect();
            line.resize(stride, 0);
            b.extend_from_slice(&line);
        }
        b
    }

    #[test]
    fn crop_of_bottom_up_bmp() {
        let img = decode_bmp(&bmp_8bit(21, 13)).unwrap();
        assert_eq!((img.width, img.height), (21, 13));
        let cut = crop(&img, 5, 4, 7).unwrap();
        for y in 0..7 {
            for x in 0..7 {
                let i = ((x + 5) + 3 * (y + 4)) % 256;
                assert_eq!(&cut.pixels[(y * 7 + x) * 4..][..4], &[i as u8, (255 - i) as u8, (i / 2) as u8, 255], "pixel {x},{y}");
            }
        }
        assert!(crop(&img, 15, 4, 7).is_none());
        assert!(crop(&img, 5, 7, 7).is_none());
    }

    #[test]
    fn top_down_bmp_is_read_in_order() {
        let mut b = bmp_8bit(4, 3);
        // flip the height sign and the rows: same picture stored top-down
        b[22..26].copy_from_slice(&(-3i32).to_le_bytes());
        let data = 14 + 40 + 1024;
        let rows: Vec<Vec<u8>> = (0..3).map(|r| b[data + 4 * r..data + 4 * r + 4].to_vec()).collect();
        for (r, row) in rows.iter().rev().enumerate() {
            b[data + 4 * r..data + 4 * r + 4].copy_from_slice(row);
        }
        let a = decode_bmp(&bmp_8bit(4, 3)).unwrap();
        let t = decode_bmp(&b).unwrap();
        assert_eq!(a.pixels, t.pixels);
    }

    #[test]
    fn square_centres_on_black() {
        let img = Rgba { width: 2, height: 4, pixels: vec![9; 2 * 4 * 4] };
        let sq = square(&img);
        assert_eq!((sq.width, sq.height), (4, 4));
        assert_eq!(&sq.pixels[0..4], &[0, 0, 0, 255]);
        assert_eq!(&sq.pixels[4..8], &[9, 9, 9, 9]);
        assert_eq!(&sq.pixels[12..16], &[0, 0, 0, 255]);
    }

    #[test]
    fn menu_texture_missing_gives_nothing() {
        let dir = std::env::temp_dir().join(format!("oxiii-menu-logo-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        assert!(logo_from_menu_texture(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ico_holds_every_size() {
        let logo = Rgba { width: 200, height: 200, pixels: vec![200; 200 * 200 * 4] };
        let ico = build_ico(&logo).unwrap();
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 4, 0]);
        let dims: Vec<u8> = (0..4).map(|i| ico[6 + 16 * i]).collect();
        assert_eq!(dims, vec![0, 48, 32, 16]);
        let off = u32::from_le_bytes(ico[18..22].try_into().unwrap()) as usize;
        assert_eq!(&ico[off..off + 4], b"\x89PNG");
    }

    #[test]
    fn missing_picture_gives_no_icon() {
        let dir = std::env::temp_dir().join(format!("oxiii-shortcut-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("help"));
        assert!(write_logo_icon(&dir, &dir.join("data"), LogoSource::HelpBitmap, true, "XIII.ico").is_none());
        // too small a picture: no icon either
        std::fs::write(dir.join("help").join("xiiilogo.BMP"), bmp_8bit(64, 64)).unwrap();
        assert!(logo_bmp(&dir).is_some());
        assert!(write_logo_icon(&dir, &dir.join("data"), LogoSource::HelpBitmap, true, "XIII.ico").is_none());
        // a usable picture in Help: the menu logo is missing, the bitmap is used instead
        std::fs::write(dir.join("help").join("xiiilogo.BMP"), bmp_8bit(640, 448)).unwrap();
        assert_eq!(logo_with_fallback(&dir, LogoSource::MenuTexture, true).map(|(s, _)| s), Some(LogoSource::HelpBitmap));
        assert!(logo_with_fallback(&dir, LogoSource::MenuTexture, false).is_none());
        assert!(write_logo_icon(&dir, &dir.join("data"), LogoSource::MenuTexture, true, "XIII.ico").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn shortcut_is_written() {
        let dir = std::env::temp_dir().join(format!("oxiii-shortcut-lnk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lnk = dir.join("OpenXIII.lnk");
        let exe = std::env::current_exe().unwrap();
        create_shortcut(&lnk, &exe, None).unwrap();
        let bytes = std::fs::read(&lnk).unwrap();
        assert_eq!(&bytes[0..4], &[0x4C, 0, 0, 0]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
