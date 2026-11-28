//! UViewport::SetDrag and UGameEngine::MouseDelta: the viewport's "drag"
//! (Viewport+0xC4). ResizeViewport sets it to 1 in fullscreen and ViewportWndProc / ResizeViewport in windowed mode reset it
//! to 0; each change goes to MouseDelta, which takes or releases the mouse capture (Client->CaptureMouse,
//! [WinDrv.WindowsClient] CaptureMouse).

/// UViewport::MOUSE_FirstHit and MOUSE_LastRelease.
pub const MOUSE_FIRST_HIT: u32 = 8;
pub const MOUSE_LAST_RELEASE: u32 = 0x10;

/// What MouseDelta reads from the engine, the client and the viewport.
#[derive(Debug, Clone, Copy, Default)]
pub struct StateMouseDelta {
    /// Engine->Client (+0x4C) non-null.
    pub client: bool,
    /// Client->Viewports.Num() (+0x2C).
    pub viewports: i32,
    /// GLevel (engine +0x1B0) non-null.
    pub glevel: bool,
    /// Viewport+0x38: UPlayer bits (bit 1: bShowWindowsMouse).
    pub bits_viewport: u32,
    /// Client->CaptureMouse (+0x38).
    pub capture_mouse: u32,
}

/// MouseDelta's calls, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDrag {
    /// Client->Viewports(0)->IsFullscreen() (vtable +0x84): returns non-zero in fullscreen.
    IsFullscreen,
    /// GLevel->GetLevelInfo()->Pauser (+0x36C): returns non-null if the game is paused.
    Pauser,
    /// Viewport->SetMouseCapture (vtable +0xB8).
    SetMouseCapture(u32, u32, u32),
}

/// UGameEngine::MouseDelta; DX and DY are not read. On the first press (MOUSE_FirstHit), outside fullscreen
/// and outside pause, the mouse is captured if bShowWindowsMouse is false; otherwise, on the last release
/// (MOUSE_LastRelease), it is released if Client->CaptureMouse is false (the client is read without a check: never null
/// in game).
pub fn mouse_delta(e: &StateMouseDelta, buttons_list: u32, host: &mut dyn FnMut(CallDrag) -> u32) {
    if buttons_list & MOUSE_FIRST_HIT != 0
        && e.client
        && e.viewports >= 1
        && e.glevel
        && host(CallDrag::IsFullscreen) == 0
        && host(CallDrag::Pauser) == 0
    {
        if e.bits_viewport & 2 == 0 {
            host(CallDrag::SetMouseCapture(1, 1, 1));
            return;
        }
    }
    if buttons_list & MOUSE_LAST_RELEASE != 0 && e.capture_mouse == 0 {
        host(CallDrag::SetMouseCapture(0, 0, 0));
    }
}

/// UViewport::SetDrag: sets Viewport+0xC4 and returns the old value; when GIsRunning, a change from
/// 0 to non-zero calls Engine->MouseDelta(Viewport, MOUSE_FirstHit, 0, 0), a change from non-zero to 0
/// MouseDelta(Viewport, MOUSE_LastRelease, 0, 0).
pub fn set_drag(drag: &mut u32, fresh_list_new: u32, gis_running: bool, mouse_delta: &mut dyn FnMut(u32)) -> u32 {
    let prior = *drag;
    *drag = fresh_list_new;
    if gis_running {
        if fresh_list_new != 0 {
            if prior == 0 {
                mouse_delta(MOUSE_FIRST_HIT);
            }
        } else if prior != 0 {
            mouse_delta(MOUSE_LAST_RELEASE);
        }
    }
    prior
}

