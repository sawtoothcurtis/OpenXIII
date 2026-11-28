//! The player's colour filter (PlayerController.FilterColor, FilterColorWanted, FilterColorSpeed,
//! FilterInternalInc), reimplemented from FCameraSceneNode::Render,
//! after FLevelSceneNode::Render and the screen flash, before Apply2DEffect and the HUD.
//!
//! AlarmFilter (SSH102b: red alarm of the self-destruct), BeachFinalFall… only set
//! FilterColorWanted: it is the engine that slides FilterColor towards it, every
//! drawn frame, and the script waits for equality (AlarmFilter.STA_FilterOn.Tick) to move to the
//! next colour.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::{ObjId, Value};
use crate::Vm;

/// FrameBufferBlending (+0x50) of the colour filter's static FinalBlend (created at the first frame by
/// FCameraSceneNode::Render): FB_Modulate.
pub const MODE_FINALBLEND_FILTER: u8 = 1;
/// Same for the screen flash: FB_AlphaBlend.
pub const MODE_FINALBLEND_FLASH: u8 = 2;

/// Screen flash of FCameraSceneNode::Render: if FlashScale.X < 1, colour (B, G, R, A) of the DrawTile of the
/// whole viewport, FColor(FPlane(FlashFog.X, .Y, .Z, 1 − FlashScale.X)) with each value clamped to [0, 1];
/// 24-bit FPU. A NaN becomes 1 after the two (unordered) comparisons.
pub fn screen_flash_tile(scale_x: f32, fog: [f32; 3]) -> Option<[u8; 4]> {
    if !(scale_x < 1.0) {
        return None;
    }
    // x87 comparisons: `v < 0` gives 0; otherwise `v < 1` keeps v; everything else, NaN included, gives 1.
    let clamp = |v: f32| {
        if v < 0.0 {
            0.0
        } else if v < 1.0 {
            v
        } else {
            1.0
        }
    };
    let s = clamp(scale_x);
    let byte = openxiii_assets::lighting::plane_byte;
    Some([byte(clamp(fog[2])), byte(clamp(fog[1])), byte(clamp(fog[0])), byte(1.0 - s)])
}

/// (_ftol): truncation, 64 bits returned in EDX:EAX.
fn ftol(x: f32) -> i64 {
    x as i64
}

/// Unreal colour (struct Color: B, G, R, A) as bytes in memory order.
pub(crate) fn bgra(v: &Value) -> [u8; 4] {
    match v {
        Value::Struct(_, f) if f.len() >= 4 => [f[0].as_byte(), f[1].as_byte(), f[2].as_byte(), f[3].as_byte()],
        _ => [0; 4],
    }
}

/// DrawTile of the filter: colour (B, G, R, A) of FilterColor unless neutral grey (R, G, B equal to 0x80).
pub(crate) fn filter_tile(cur: [u8; 4]) -> Option<[u8; 4]> {
    (cur[2] != 0x80 || cur[1] != 0x80 || cur[0] != 0x80).then_some(cur)
}

/// One channel of FilterColor towards that of FilterColorWanted, by at most `step` (32-bit
/// comparisons, as in the machine code); otherwise it takes the desired value.
fn approach(cur: u8, want: u8, step: u32) -> u8 {
    let (c, w) = (cur as u32, want as u32);
    if c < w {
        if c + step < w && c + step <= 0xFF {
            cur.wrapping_add(step as u8)
        } else {
            want
        }
    } else if c > w {
        if c.wrapping_sub(step) > w && c >= step {
            cur.wrapping_sub(step as u8)
        } else {
            want
        }
    } else {
        cur
    }
}

/// Offset (a, b) of the shake frame (FCameraSceneNode::Render): None when the
/// shake is over (t outside [0, ShakeFrameTime[), otherwise k = 1 − t / ShakeFrameTime, a = k × MaxShakeFrame.X ×
/// sin(17 t), b = k × MaxShakeFrame.Y × sin(21 t).
pub(crate) fn shake_offsets(time: f32, max: [f32; 3], now: f32) -> Option<(f32, f32)> {
    let t = now - max[2];
    if t < 0.0 || !(t < time) {
        return None;
    }
    let k = 1.0 - t / time;
    // appSin on the double, result stored as a double (FSTP qword) then
    // multiplied (FMUL qword): a single 24-bit rounding, that of the product.
    let a = crate::ai_move::fmul_d(k * max[0], ((t * 17.0) as f64).dsin());
    let b = crate::ai_move::fmul_d(k * max[1], ((t * 21.0) as f64).dsin());
    Some((a, b))
}

impl Vm {
    /// UViewport::Lock: CurrentTime (+0x140, appSeconds) and, at +0x148,
    /// the difference with the CurrentTime of the previous lock (0 at the start, UViewport::UViewport).
    pub(crate) fn viewport_lock(&mut self, viewport: ObjId) -> f32 {
        // Logical boundary of the current drawing. This success does not assert a
        // GPU acquisition: depending on the caller, it happens before or after here.
        self.viewport_lock_device(viewport, false, false, |_| true).delta
    }

    /// Per-viewport state, then a call to the single
    /// RenDev::Lock boundary. A null answer does not remove the previous writes.
    pub(crate) fn viewport_lock_device(
        &mut self,
        viewport: ObjId,
        hit_data: bool,
        hit_size: bool,
        device: impl FnOnce(&crate::client::ViewportLockCall) -> bool,
    ) -> crate::client::ViewportLockOutcome {
        let now = self.app_seconds;
        let last = self.viewport_times.insert(viewport, now).unwrap_or(0.0);
        self.viewport_delta = (now - last) as f32;
        // The old timing call also accepts an identifier without an object.
        let actor = self.objects.get(viewport).and_then(|o| o.as_ref())
            .and_then(|_| self.propr(viewport, "Actor").as_object());
        let rend = actor.map_or(0, |pc| self.propr(pc, "RendMap").as_int());
        let call = {
            let state = self.engine.client.viewport_locks.entry(viewport).or_default();
            state.current_time = now;
            state.delta = self.viewport_delta;
            state.hit = hit_data;
            state.render_flags = if rend == 2 || rend == 4 { 0x10000 } else { 0 };
            state.frame_count = state.frame_count.wrapping_add(1);
            crate::client::ViewportLockCall { viewport, hit_data, hit_size, state: *state }
        };
        let accepted = device(&call);
        self.engine.client.viewport_locks.get_mut(&viewport).unwrap().interface = accepted;
        crate::client::ViewportLockOutcome { accepted, delta: self.viewport_delta }
    }

    pub(crate) fn viewport_lock_state(&self, viewport: ObjId) -> Option<crate::client::ViewportLockState> {
        self.engine.client.viewport_locks.get(&viewport).copied()
    }

    /// UGameEngine::Draw, camera zone with bForceClear: the frame is
    /// cleared to the view volume's DistanceFogColor if it has bDistanceFog (+0x2D4, colour +0x2D8),
    /// otherwise to the zone's. The volume is FLevelSceneNode::Init's: Viewport->Actor->
    /// ViewTarget, its HeadVolume if it is a pawn, otherwise its PhysicsVolume. Returns the
    /// volume's colour (R, G, B) when it replaces the zone's. Without bForceClear: black, and the
    /// colour is only cleared with several viewports (Client+0x78 > 1).
    pub fn view_volume_clear(&self, controller: Option<ObjId>) -> Option<[u8; 3]> {
        let volume = self.view_volume(controller).filter(|&v| self.propr(v, "bDistanceFog").as_bool())?;
        let c = bgra(&self.prop(volume, "DistanceFogColor"));
        Some([c[2], c[1], c[0]])
    }

    /// FLevelSceneNode::Init (outside the editor): view volume (+0x1D8),
    /// Viewport->Actor->ViewTarget (+0x3A4), its HeadVolume if it is a pawn, otherwise its
    /// PhysicsVolume.
    pub fn view_volume(&self, controller: Option<ObjId>) -> Option<ObjId> {
        let target = controller.and_then(|c| self.propr(c, "ViewTarget").as_object()).filter(|&t| self.objects.get(t).is_some_and(|o| o.is_some()))?;
        if self.is_a(target, "Pawn") { self.prop(target, "HeadVolume") } else { self.prop(target, "PhysicsVolume") }.as_object()
    }

    /// FLevelSceneNode::Render, scene without a parent (+8 null): if the PlayerController
    /// does not have UseVignetteFilter (+0x5B0), has a ViewTarget, SHOW_DistanceFog (ShowFlags 0x80000)
    /// and the view volume has bDistanceFog (+0x2D4), SetDistanceFog(GEdShowFogInViewports
    /// (1, Core.dll) and not wireframe, DistanceFogStart +0x2DC, DistanceFogEnd +0x2E0,
    /// DistanceFogColor +0x2D8); otherwise the zone's fog. Start, end, R, G, B.
    pub fn view_volume_fog(&self, controller: Option<ObjId>) -> Option<(f32, f32, [u8; 3])> {
        let pc = controller.filter(|&c| self.objects.get(c).is_some_and(|o| o.is_some()))?;
        if self.propr(pc, "UseVignetteFilter").as_byte() != 0 || self.propr(pc, "ShowFlags").as_int() & 0x80000 == 0 {
            return None;
        }
        let volume = self.view_volume(Some(pc)).filter(|&v| self.propr(v, "bDistanceFog").as_bool())?;
        let c = bgra(&self.prop(volume, "DistanceFogColor"));
        Some((self.propr(volume, "DistanceFogStart").as_float(), self.propr(volume, "DistanceFogEnd").as_float(), [c[2], c[1], c[0]]))
    }

    /// Main viewport without split screen (same sequence as `draw_viewport`): UViewport::Lock
    /// then, after the scene and unless SetOnlyPostRender, the frame shake and the colour
    /// filter of `pc`; colour of the
    /// DrawTile in B, G, R, A.
    pub fn main_camera_filter(&mut self, pc: Option<ObjId>) -> Option<[u8; 4]> {
        let viewport = self.viewport_object(0)?;
        let lock = self.viewport_lock_device(viewport, false, false, |_| true);
        if !lock.accepted { return None; }
        let delta = lock.delta;
        match pc {
            Some(pc) if !self.only_post_render && self.objects.get(pc).is_some_and(|o| o.is_some()) => {
                self.camera_shake_frame(pc);
                self.camera_filter(pc, delta)
            }
            _ => None,
        }
    }

    /// FCameraSceneNode::Render: non-zero ShakeFrameTime (XIIIPlayerPawn.TakeDamage,
    /// grenade or rocket in single-player): t = Level.TimeSeconds − MaxShakeFrame.Z (start date);
    /// outside [0, ShakeFrameTime[ the shake stops (ShakeFrameTime = 0), otherwise
    /// k = 1 − t / ShakeFrameTime, a = k × MaxShakeFrame.X × sin(17 t),
    /// b = k × MaxShakeFrame.Y × sin(21 t) (24-bit FPU) and UCanvas::DrawFrame(b, b + 1, a,
    /// a + 1) on the viewport's Canvas, whose triangles come before the HUD's.
    pub(crate) fn camera_shake_frame(&mut self, pc: ObjId) {
        let time = self.propr(pc, "ShakeFrameTime").as_float();
        if time == 0.0 {
            return;
        }
        let now = self.level.map(|l| self.propr(l, "TimeSeconds").as_float()).unwrap_or(0.0);
        let max = self.propr(pc, "MaxShakeFrame").as_vector();
        let Some((a, b)) = shake_offsets(time, max, now) else {
            self.set_prop(pc, "ShakeFrameTime", Value::Float(0.0));
            return;
        };
        if let Some(c) = self.canvas.filter(|&c| self.objects.get(c).is_some_and(|o| o.is_some())) {
            self.canvas_draw_frame(c, b, b + 1.0, a, a + 1.0);
        }
    }

    /// FCameraSceneNode::Render for Viewport->Actor `pc`, `delta` being
    /// Viewport+0x148: if FilterColor is not neutral grey (R, G, B = 128, the alpha is not
    /// read), DrawTile of the whole viewport with this colour (FinalBlend:
    /// FrameBufferBlending 1 = FB_Modulate, WhiteTexture) — the returned colour, the one before the
    /// update. Then, if FilterColor ≠ FilterColorWanted (the 4 bytes) and FilterColorSpeed
    /// > 0: v = FilterColorSpeed × 65536 × delta + FilterInternalInc (unsigned), in 32-bit
    /// floats (24-bit FPU); _ftol(v) gives the step (bits 8 and above) and the remainder
    /// FilterInternalInc (low 8 bits); R, G then B approach the desired colour by the step, the alpha
    /// does not move.
    pub(crate) fn camera_filter(&mut self, pc: ObjId, delta: f32) -> Option<[u8; 4]> {
        let cur = bgra(&self.prop(pc, "FilterColor"));
        let want = bgra(&self.prop(pc, "FilterColorWanted"));
        let speed = self.propr(pc, "FilterColorSpeed").as_float();
        let inc = self.propr(pc, "FilterInternalInc").as_int();
        let tile = filter_tile(cur);
        if cur != want && speed > 0.0 {
            let mut acc = inc as f32;
            if inc < 0 {
                acc += 4_294_967_296.0;
            }
            let v = speed * 65536.0 * delta + acc;
            let r = ftol(v) as u32;
            let step = r >> 8;
            let mut next = cur;
            next[2] = approach(cur[2], want[2], step);
            next[1] = approach(cur[1], want[1], step);
            next[0] = approach(cur[0], want[0], step);
            let color = Value::Struct("object.color".into(), next.iter().map(|&b| Value::Byte(b)).collect());
            self.set_prop(pc, "FilterColor", color);
            self.set_prop(pc, "FilterInternalInc", Value::Int((r & 0xFF) as i32));
        }
        tile
    }
}

#[cfg(test)]
mod tests {
    use super::approach;

    #[test]
    fn channels_like_the_machine_code() {
        // Rising: whole step as long as we stay below the target, otherwise the target.
        assert_eq!(approach(128, 222, 10), 138);
        assert_eq!(approach(220, 222, 10), 222);
        // Falling: same, and the target if the step goes past the value.
        assert_eq!(approach(128, 22, 10), 118);
        assert_eq!(approach(25, 22, 10), 22);
        assert_eq!(approach(5, 0, 10), 0);
        assert_eq!(approach(50, 50, 10), 50);
        // Step of 300: the target directly.
        assert_eq!(approach(128, 222, 300), 222);
    }
}

