//! UWindowsViewport::ResizeViewport (flags, NewX, NewY), outside the editor: the state of the
//! viewport and of the client that it writes (viewport sizes, fullscreen flags, window rectangle,
//! sizes saved in windowed and fullscreen mode in [WinDrv.WindowsClient]) and the calls that are not
//! Windows ones (virtual functions of the viewport and of the console, SaveConfig, SetTopness, appFailAssert). The
//! Win32 calls (SetFocus, hotkeys, styles, MoveWindow…) belong to the platform: they are not reproduced.

/// A call that is not Win32, in the original's order.
#[derive(Debug, Clone, PartialEq)]
pub enum ResizeCall {
    /// Virtual function of the viewport (vtable +offset) with its arguments.
    Viewport(u32, Vec<u32>),
    /// Virtual function of the console (the client's GEngine + 0x50): +0xF4 returns the state, +0x74 restores it.
    Console(u32, Vec<u32>),
    SaveConfig(u32, u32),
    SetTopness,
    /// appFailAssert(expression, line).
    FailAssert(&'static str, u32),
}

impl std::fmt::Display for ResizeCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list_value = |f: &mut std::fmt::Formatter<'_>, v: &[u32]| v.iter().try_for_each(|x| write!(f, " {x:x}"));
        match self {
            ResizeCall::Viewport(o, a) => {
                write!(f, "vp {o:x}")?;
                list_value(f, a)
            }
            ResizeCall::Console(o, a) => {
                write!(f, "con {o:x}")?;
                list_value(f, a)
            }
            ResizeCall::SaveConfig(a, b) => write!(f, "SaveConfig {a:x} {b:x}"),
            ResizeCall::SetTopness => write!(f, "SetTopness"),
            ResizeCall::FailAssert(e, l) => write!(f, "appFailAssert {e} {l:x}"),
        }
    }
}

/// What ResizeViewport reads and writes of the viewport and of the client.
#[derive(Debug, Clone, Default)]
pub struct ResizeViewport {
    /// Viewport + 0x70 (RenDev) non-null and its field +0x48.
    pub rendev: bool,
    pub rendev_48: bool,
    /// UObject::GetFlags of the viewport: bit 0x800000.
    pub object_flags: u32,
    /// Window open (non-null HWND).
    pub has_hwnd: bool,
    /// The render device's class is called OpenGLRenderDevice.
    pub rendev_is_opengl: bool,
    /// Actor + 0x390: bit 0x200.
    pub actor_flags: u32,
    /// The console of the client's GEngine (client + 0x28 + 0x50) exists.
    pub console: bool,
    /// Client + 0xCC non-null (client window to destroy).
    pub client_window: bool,
    // Written:
    pub size_x: u32,
    pub size_y: u32,
    pub flags: u32,
    pub f14c: u32,
    pub f1ac: u32,
    pub fbc: u32,
    pub f1e0: u32,
    /// Rectangle +0x1D0 (x, y, width, height).
    pub rect: [u32; 4],
    /// Client + 0x3C, 0x40 (windowed), 0x44, 0x48 (fullscreen).
    pub sizes: [u32; 4],
}

/// ResizeViewport; `host` runs the calls, `window_rect` gives GetWindowRect (x, y, width, height). Returns 0 or 1.
pub fn resize_viewport(
    v: &mut ResizeViewport,
    flags: u32,
    new_x: i32,
    new_y: i32,
    host: &mut dyn FnMut(ResizeCall) -> u32,
    window_rect: &dyn Fn() -> [u32; 4],
) -> u32 {
    let full = flags & 1 != 0;
    // Active render device and window request: only the windowed render device is reloaded.
    if v.rendev && v.rendev_48 && !full {
        if v.object_flags & 0x80_0000 == 0 {
            host(ResizeCall::Viewport(0xC0, vec![0x1111_0608, 0xFFFF_FFFF, 0xFFFF_FFFF, 0]));
            if !v.rendev {
                host(ResizeCall::FailAssert("RenDev", 0xA80));
            }
        }
        return 0;
    }
    let saved_console = if v.console && v.object_flags & 0x80_0000 == 0 { host(ResizeCall::Console(0xF4, vec![])) } else { 0 };
    // Current rectangle: the saved one, or the window's.
    let mut rect = v.rect;
    if v.has_hwnd && v.flags & 1 == 0 && flags & 0x20 == 0 {
        rect = window_rect();
    }
    let (mut x, mut y) = (new_x, new_y);
    if x == -1 {
        if full {
            x = v.sizes[2] as i32;
            y = v.sizes[3] as i32;
        } else {
            x = v.sizes[0] as i32;
            y = v.sizes[1] as i32;
        }
    }
    if x < 0 {
        host(ResizeCall::FailAssert("NewX>=0", 0xA99));
    }
    if y < 0 {
        host(ResizeCall::FailAssert("NewY>=0", 0xA9A));
    }
    let x = ((x as u32).wrapping_add(1)) & !1;
    let y = y as u32;
    let old_flags = v.flags;
    if old_flags & 1 != 0 {
        // Leaves fullscreen: the saved rectangle comes back.
        rect = v.rect;
        v.fbc = v.f1e0;
        host(ResizeCall::SetTopness);
        host(ResizeCall::Viewport(0x80, vec![0]));
    }
    if full && old_flags & 1 == 0 {
        v.rect = rect;
        v.f1e0 = v.fbc;
        if v.client_window {
            // DestroyWindow of the client's window, update of the viewport window's style.
        }
        v.f1ac = 1;
    }
    if flags & 2 == 0 {
        v.f14c = 0;
    }
    v.flags = flags & !0x40;
    v.size_y = y;
    v.size_x = x;
    if !full && old_flags & 1 != 0 {
        host(ResizeCall::Viewport(0xB8, vec![0, 0, 0]));
    }
    if full {
        host(ResizeCall::Viewport(0x80, vec![1]));
        host(ResizeCall::Viewport(0xB8, vec![1, 1, 0]));
    } else if flags & 2 == 0 && flags & 0x20 == 0 {
        host(ResizeCall::Viewport(0x80, vec![0]));
    }
    if saved_console != 0 && v.console && host(ResizeCall::Console(0xF4, vec![])) != saved_console {
        host(ResizeCall::Console(0x74, vec![saved_console]));
    }
    host(ResizeCall::Viewport(0x90, vec![]));
    if v.rendev {
        if x != 0 && y != 0 {
            if full {
                v.sizes[2] = x;
                v.sizes[3] = y;
            } else {
                v.sizes[0] = x;
                v.sizes[1] = y;
            }
        }
        host(ResizeCall::SaveConfig(0x4000, 0));
    }
    1
}

/// WinDrv's standard resolutions (static array): area (float), width, height.
const RESOLUTIONS: [(u32, i32, i32); 7] = [
    (0x4896_0000, 640, 480),
    (0x48EA_6000, 800, 600),
    (0x4940_0000, 1024, 768),
    (0x4973_0000, 1152, 864),
    (0x4996_0000, 1280, 960),
    (0x49EA_6000, 1600, 1200),
    (0x4D37_1B00, 16000, 12000),
];

/// "Fits in the desktop" flags of the array, computed on the first call of function.
#[derive(Debug, Clone, Default)]
pub struct ResTable {
    fit: Option<[bool; 7]>,
}

impl ResTable {
    /// Function (called by OpenWindow and SetRenDevRes): brings (width, height) to the standard
    /// resolution whose area is the closest (halfway between neighbouring areas), among those that fit in
    /// the desktop `desktop` (GetDeviceCaps HORZRES, VERTRES of the first call); larger than all of them, the size
    /// stays; with no smaller resolution that fits, returns 0 x 0.
    pub fn choose(&mut self, w: &mut i32, h: &mut i32, desktop: (i32, i32)) {
        let fit = *self.fit.get_or_insert_with(|| std::array::from_fn(|i| RESOLUTIONS[i].1 <= desktop.0 && RESOLUTIONS[i].2 <= desktop.1));
        let (wc, hc) = ((*w).min(16000), (*h).min(12000));
        let area = wc.wrapping_mul(hc) as f32;
        for i in 0..7 {
            if !fit[i] {
                (*w, *h) = if i == 0 { (0, 0) } else { (RESOLUTIONS[i - 1].1, RESOLUTIONS[i - 1].2) };
                return;
            }
            // The array's flag (1) acts as the next area for the last entry.
            let next_value = if i < 6 { f32::from_bits(RESOLUTIONS[i + 1].0) } else { f32::from_bits(1) };
            let middle = (f32::from_bits(RESOLUTIONS[i].0) + next_value) * 0.5;
            if middle > area {
                (*w, *h) = (RESOLUTIONS[i].1, RESOLUTIONS[i].2);
                return;
            }
        }
    }
}

/// One call of ToggleFullscreen, EndFullscreen or SetRenDevRes.
#[derive(Debug, Clone, PartialEq)]
pub enum FsCall {
    /// Game log (GLog->Logf).
    Log(&'static str),
    /// Virtual function of the viewport (vtable +offset) with its arguments.
    Viewport(u32, Vec<u32>),
    /// Viewport vtable +0xC0: requested render device, width, height, fullscreen.
    RenderDevice(&'static str, i32, i32, i32),
    /// Render device vtable +0x78 (SetRes): width, height, fullscreen.
    SetRes(i32, i32, i32),
    /// GEngine vtable +0xBC.
    Engine,
}

impl std::fmt::Display for FsCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsCall::Log(_) => Ok(()),
            FsCall::Viewport(o, a) => {
                write!(f, "vp {o:x}")?;
                a.iter().try_for_each(|x| write!(f, " {x:x}"))
            }
            FsCall::RenderDevice(n, w, h, fs) => write!(f, "vp c0 {n} {:x} {:x} {:x}", *w as u32, *h as u32, *fs as u32),
            FsCall::SetRes(w, h, fs) => write!(f, "rd 78 V {:x} {:x} {:x}", *w as u32, *h as u32, *fs as u32),
            FsCall::Engine => write!(f, "gen bc"),
        }
    }
}

/// What ToggleFullscreen, EndFullscreen and SetRenDevRes read of the viewport and its client.
#[derive(Debug, Clone, Default)]
pub struct FullscreenViewport {
    pub rendev: bool,
    pub rendev_48: bool,
    /// Viewport + 0x1A8.
    pub flags: u32,
    /// Actor + 0x390.
    pub actor_flags: u32,
    /// Client + 0x3C, 0x40: window size.
    pub sizes: [i32; 2],
}

const WINDOWED: &str = "ini:Engine.Engine.WindowedRenderDevice";
const GAME_DEVICE: &str = "ini:Engine.Engine.GameRenderDevice";

impl ResTable {
    /// UWindowsViewport::SetRenDevRes: in windowed mode (`fullscreen` zero) and other than -1, brings (width,
    /// height) to a standard resolution, then the render device's SetRes; on success the sizes are returned.
    pub fn set_ren_dev_res(&mut self, w: &mut i32, h: &mut i32, fullscreen: i32, desktop: (i32, i32), host: &mut dyn FnMut(FsCall) -> u32) -> bool {
        let (mut nw, mut nh) = (*w, *h);
        if fullscreen == 0 && nw != -1 && nh != -1 {
            self.choose(&mut nw, &mut nh, desktop);
        }
        if host(FsCall::SetRes(nw, nh, fullscreen)) != 0 {
            *w = nw;
            *h = nh;
            true
        } else {
            false
        }
    }

    /// UWindowsViewport::EndFullscreen, outside the editor.
    pub fn end_fullscreen(&mut self, v: &mut FullscreenViewport, desktop: (i32, i32), host: &mut dyn FnMut(FsCall) -> u32) {
        host(FsCall::Log("EndFullscreen"));
        if v.rendev {
            if v.rendev_48 {
                if host(FsCall::RenderDevice(WINDOWED, -1, -1, 0)) != 0 {
                    v.rendev = true;
                }
            } else if v.flags & 0x10 != 0 {
                let (mut w, mut h) = (v.sizes[0], v.sizes[1]);
                if self.set_ren_dev_res(&mut w, &mut h, 0, desktop, host) {
                    v.sizes = [w, h];
                }
            } else if v.flags & 0x80 != 0 {
                let (mut w, mut h) = (-1, -1);
                self.set_ren_dev_res(&mut w, &mut h, 0, desktop, host);
            }
        }
        host(FsCall::Engine);
        host(FsCall::Viewport(0x90, vec![]));
    }

    /// UWindowsViewport::ToggleFullscreen, outside the editor.
    pub fn toggle_fullscreen(&mut self, v: &mut FullscreenViewport, desktop: (i32, i32), host: &mut dyn FnMut(FsCall) -> u32) {
        if v.flags & 1 != 0 {
            self.end_fullscreen(v, desktop, host);
            host(FsCall::Viewport(0xB8, vec![1, 1, 0]));
            return;
        }
        if v.actor_flags & 0x200 != 0 {
            return;
        }
        host(FsCall::Log("AttemptFullscreen"));
        for (name_value, fs) in [(GAME_DEVICE, 1), (WINDOWED, 1), (WINDOWED, 0)] {
            if host(FsCall::RenderDevice(name_value, -1, -1, fs)) != 0 {
                v.rendev = true;
            }
            if v.rendev {
                return;
            }
        }
    }
}

/// A call of OpenWindow that is not Win32.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenCall {
    /// appFailAssert(expression, line).
    FailAssert(&'static str, u32),
    /// Viewport vtable +0xC0: requested render device, width, height, fullscreen; the host returns true if
    /// the viewport then has a render device.
    RenderDevice(&'static str, i32, i32, i32),
    /// GMalloc->Malloc of the temporary viewport's frame memory (bytes); the host returns the block.
    Malloc(u32),
    /// Render device vtable +0x84 (the viewport as argument).
    RenderDeviceOpen,
    /// Viewport vtable +0x90.
    Viewport90,
    /// Game log (GLog->Logf).
    Log(&'static str),
}

impl std::fmt::Display for OpenCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenCall::FailAssert(e, l) => write!(f, "appFailAssert {e} {l:x}"),
            OpenCall::RenderDevice(n, w, h, fs) => write!(f, "vp c0 {n} {:x} {:x} {:x}", *w as u32, *h as u32, *fs as u32),
            OpenCall::Malloc(_) | OpenCall::Log(_) => Ok(()),
            OpenCall::RenderDeviceOpen => write!(f, "rd 84 V"),
            OpenCall::Viewport90 => write!(f, "vp 90"),
        }
    }
}

/// Boundaries needed by the real host, separate from the historical non-Win32 traces.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenHostCall {
    /// Window creation; an error stops the body before it is published.
    CreateWindow { parent: u32, width: i32, height: i32 },
    Call(OpenCall),
}

/// What OpenWindow reads and writes of the viewport and its client.
#[derive(Debug, Clone, Default)]
pub struct OpenViewport {
    /// The viewport's name is "VisibiltyViewport" (editor).
    pub visibility_viewport: bool,
    pub actor: bool,
    pub lock: u32,
    pub rendev: bool,
    /// Actor + 0x390: bit 0x200.
    pub actor_flags: u32,
    /// WWindow: non-null handle (window already created).
    pub has_hwnd: bool,
    pub f19c: u32,
    pub f1a8: u32,
    pub fb4: u32,
    pub size_x: i32,
    pub size_y: i32,
    pub f14c: u32,
    /// Client + 0x94 (fullscreen requested) and + 0x3C, 0x40.
    pub client_f94: bool,
    pub sizes: [i32; 2],
    /// Improved build: the saved window size is kept as it is instead of being brought to a
    /// standard resolution (improved/storage.rs, window_size_kept). Always false in the faithful
    /// build, as in the original.
    pub keep_size: bool,
}

impl ResTable {
    /// UWindowsViewport::OpenWindow, outside the editor and with a 32-bit desktop: `hwnd_param` is the result of
    /// Parse(command line, "HWND="), `nohard` that of ParseParam("nohard"). The Win32 calls (window,
    /// DirectInput) belong to the platform.
    #[allow(clippy::too_many_arguments)]
    pub fn open_window(
        &mut self,
        v: &mut OpenViewport,
        parent: u32,
        temporary: u32,
        new_x: i32,
        new_y: i32,
        desktop: (i32, i32),
        hwnd_param: Option<u32>,
        nohard: bool,
        host: &mut dyn FnMut(OpenCall) -> u32,
    ) {
        // The historical contract ignores Win32 creation and allows assertions to return.
        // This single adapter keeps exactly these answers; the real host goes through try_open_window.
        let result = self.try_open_window(v, parent, temporary, new_x, new_y, desktop, hwnd_param, nohard, &mut |call| {
            Ok::<u32, std::convert::Infallible>(match call {
                OpenHostCall::CreateWindow { .. } => 0,
                OpenHostCall::Call(call) => host(call),
            })
        });
        match result { Ok(()) => {}, Err(never) => match never {} }
    }

    /// Same OpenWindow body, with real boundaries and error propagation without a false fatal return.
    #[allow(clippy::too_many_arguments)]
    pub fn try_open_window<E>(
        &mut self, v: &mut OpenViewport, parent: u32, temporary: u32,
        new_x: i32, new_y: i32, desktop: (i32, i32), hwnd_param: Option<u32>, nohard: bool,
        host: &mut dyn FnMut(OpenHostCall) -> Result<u32, E>,
    ) -> Result<(), E> {
        if !v.actor {
            host(OpenHostCall::Call(OpenCall::FailAssert("Actor", 0x290)))?;
        }
        if v.lock != 0 {
            host(OpenHostCall::Call(OpenCall::FailAssert("!HoldCount", 0x291)))?;
        }
        let x = if new_x != -1 { ((new_x as u32).wrapping_add(1) & !1) as i32 } else { new_x };
        let (mut sw, mut sh) = (v.sizes[0], v.sizes[1]);
        if !v.keep_size {
            self.choose(&mut sw, &mut sh, desktop);
        }
        v.sizes = [sw, sh];
        let parent = if parent == 0 { hwnd_param.unwrap_or(0) } else { parent };

        if temporary != 0 {
            v.f1a8 = 2;
            v.fb4 = 2;
            v.size_x = x;
            v.size_y = new_y;
            v.f14c = host(OpenHostCall::Call(OpenCall::Malloc((x.wrapping_mul(new_y) as u32).wrapping_mul(2))))?;
            v.has_hwnd = false;
            host(OpenHostCall::Call(OpenCall::Log("Opened temporary viewport")))?;
        } else if !v.has_hwnd {
            // Window creation (Win32): the parent is kept at +0x19C.
            v.f19c = parent;
            let width = if x == -1 { v.sizes[0] } else { x };
            let height = if new_y == -1 { v.sizes[1] } else { new_y };
            host(OpenHostCall::CreateWindow { parent, width, height })?;
            v.has_hwnd = true;
            host(OpenHostCall::Call(OpenCall::Log("Opened viewport")))?;
        }
        let device = |v: &mut OpenViewport, host: &mut dyn FnMut(OpenHostCall) -> Result<u32, E>, name: &'static str, fs: i32| -> Result<(), E> {
            if host(OpenHostCall::Call(OpenCall::RenderDevice(name, x, new_y, fs)))? != 0 {
                v.rendev = true;
            }
            Ok(())
        };
        if !v.rendev && v.visibility_viewport {
            device(v, host, "Editor.VisibilityRenderDevice", 0)?;
        }
        if !v.rendev {
            if v.f1a8 & 2 != 0 {
                device(v, host, "D3DDrv.D3DRenderDevice", 0)?;
            }
            if !v.rendev {
                if !nohard {
                    device(v, host, "ini:Engine.Engine.GameRenderDevice", i32::from(v.client_f94))?;
                }
                if !v.rendev {
                    device(v, host, "ini:Engine.Engine.WindowedRenderDevice", i32::from(v.client_f94))?;
                }
            }
        }
        if !v.rendev {
            host(OpenHostCall::Call(OpenCall::FailAssert("RenDev", 0x313)))?;
        }
        host(OpenHostCall::Call(OpenCall::RenderDeviceOpen))?;
        host(OpenHostCall::Call(OpenCall::Viewport90))?;
        Ok(())
    }

}

