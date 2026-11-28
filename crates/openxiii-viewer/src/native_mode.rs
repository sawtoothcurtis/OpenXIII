//! Improved build: the screen's native resolution, the largest the Video menu offers. A driver can
//! list modes larger than the panel (scaled modes, super-resolution factors); drawn there, the
//! picture is shrunk by the screen and every text becomes soft.

/// Native resolution of `monitor`: on Windows the preferred mode of the display it shows (the
/// panel's own size), otherwise, or when Windows does not say, its size now (the desktop's, read
/// before the game changes the display mode).
pub fn of_monitor(monitor: &winit::monitor::MonitorHandle) -> Option<(u32, u32)> {
    #[cfg(windows)]
    if let Some(size) = monitor.name().and_then(|n| windows::preferred_mode(&n)) {
        return Some(size);
    }
    let s = monitor.size();
    (s.width > 0 && s.height > 0).then_some((s.width, s.height))
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;

    const QDC_ONLY_ACTIVE_PATHS: u32 = 2;
    const GET_SOURCE_NAME: u32 = 1;
    const GET_TARGET_PREFERRED_MODE: u32 = 3;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Luid {
        low: u32,
        high: i32,
    }

    /// DISPLAYCONFIG_PATH_INFO (72 bytes): only the source's and the target's adapter and id are read.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct PathInfo {
        source_adapter: Luid,
        source_id: u32,
        source_rest: [u32; 2],
        target_adapter: Luid,
        target_id: u32,
        target_rest: [u32; 9],
        flags: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ModeInfo {
        raw: [u64; 8],
    }

    #[repr(C)]
    struct Header {
        kind: u32,
        size: u32,
        adapter: Luid,
        id: u32,
    }

    /// DISPLAYCONFIG_SOURCE_DEVICE_NAME.
    #[repr(C)]
    struct SourceName {
        header: Header,
        gdi_name: [u16; 32],
    }

    /// DISPLAYCONFIG_TARGET_PREFERRED_MODE (the signal info that follows is not read).
    #[repr(C)]
    struct PreferredMode {
        header: Header,
        width: u32,
        height: u32,
        target_mode: [u64; 6],
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetDisplayConfigBufferSizes(flags: u32, paths: *mut u32, modes: *mut u32) -> i32;
        fn QueryDisplayConfig(flags: u32, paths: *mut u32, path_array: *mut PathInfo, modes: *mut u32, mode_array: *mut ModeInfo, topology: *mut c_void) -> i32;
        fn DisplayConfigGetDeviceInfo(packet: *mut Header) -> i32;
    }

    /// The preferred mode of the active display shown by the GDI device `gdi_name` (\\.\DISPLAY1).
    pub fn preferred_mode(gdi_name: &str) -> Option<(u32, u32)> {
        const _: () = assert!(std::mem::size_of::<PathInfo>() == 72 && std::mem::size_of::<ModeInfo>() == 64);
        const _: () = assert!(std::mem::size_of::<SourceName>() == 84 && std::mem::size_of::<PreferredMode>() == 80);
        let (mut n_paths, mut n_modes) = (0u32, 0u32);
        // SAFETY: plain out-parameters.
        if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes) } != 0 || n_paths == 0 {
            return None;
        }
        let empty_path = PathInfo { source_adapter: Luid::default(), source_id: 0, source_rest: [0; 2], target_adapter: Luid::default(), target_id: 0, target_rest: [0; 9], flags: 0 };
        let mut paths = vec![empty_path; n_paths as usize];
        let mut modes = vec![ModeInfo { raw: [0; 8] }; n_modes.max(1) as usize];
        // SAFETY: arrays of the sizes Windows asked for, counts updated in place.
        if unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, paths.as_mut_ptr(), &mut n_modes, modes.as_mut_ptr(), std::ptr::null_mut()) } != 0 {
            return None;
        }
        paths.truncate(n_paths as usize);
        let wanted = gdi_name.trim_end_matches('\0');
        for p in &paths {
            let mut source = SourceName {
                header: Header { kind: GET_SOURCE_NAME, size: std::mem::size_of::<SourceName>() as u32, adapter: p.source_adapter, id: p.source_id },
                gdi_name: [0; 32],
            };
            // SAFETY: a packet of the declared kind and size.
            if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0 {
                continue;
            }
            let len = source.gdi_name.iter().position(|&c| c == 0).unwrap_or(32);
            if !String::from_utf16_lossy(&source.gdi_name[..len]).eq_ignore_ascii_case(wanted) {
                continue;
            }
            let mut preferred = PreferredMode {
                header: Header { kind: GET_TARGET_PREFERRED_MODE, size: std::mem::size_of::<PreferredMode>() as u32, adapter: p.target_adapter, id: p.target_id },
                width: 0,
                height: 0,
                target_mode: [0; 6],
            };
            // SAFETY: as above.
            if unsafe { DisplayConfigGetDeviceInfo(&mut preferred.header) } == 0 && preferred.width > 0 && preferred.height > 0 {
                return Some((preferred.width, preferred.height));
            }
        }
        None
    }
}
