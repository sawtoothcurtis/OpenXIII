//! 2D screen effects of FD3DRenderInterface (D3DDrv.dll): poison, blur / sharpen, injury (and
//! the underwater window), set by the LevelInfo natives and by APlayerController::Tick, then
//! applied by Apply2DEffect (vtable +0x94), which FCameraSceneNode::Render
//! calls after the scene and before the HUD.
//!
//! As long as an effect's intensity is not zero, the scene is drawn into the
//! device's 512 × 512 render texture (surface +0x14A0, viewport 512 ×
//! 512); the effect then redraws it into the screen buffer (+0x148C) as full-screen quads
//! or as a deformed grid (poison). The viewer draws the `FxPass` passes that
//! `apply_2d_effect` returns. The fields follow the device's (+0xD3C…+0xD6C).
//!
//! UGameEngine::Draw also sets, every frame, the water effect of the view's volume
//! (vtable +0x9C, drawn) and the flash of the view's zone
//! (vtable +0x90, drawn); both send the scene into the
//! texture and only apply to the frame in which they are set.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::Value;

/// GEngine->DeltaSeconds zero: `apply_2d_effect` receives it from the engine.
const RAMP_KICK: f32 = 0.0001;

/// Vertex of an effect pass: position in [−1, 1]² (y upwards, orthogonal projection), coordinates of stages 0 and 1, D3DCOLOR colour (0xAARRGGBB).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FxVertex {
    pub pos: [f32; 2],
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    pub color: u32,
}

/// Render surface of IDirect3DDevice8::SetRenderTarget: scene (+0x14A0, 512 × 512, the one
/// that is set when Apply2DEffect starts), small flash texture (+0x14A4, 128 × 128,
/// without depth) or screen buffer (+0x148C, viewport SizeX × SizeY).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxTarget {
    Scene,
    Small,
    Back,
}

/// Stage 0 texture: none, the scene (+0x1494) or the small one (+0x1498).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxTex {
    None,
    Scene,
    Small,
}

/// Effect pass, in machine-code order. The `Clear`, `Quad` and `Mesh` passes read the
/// scene texture.
#[derive(Debug, Clone, PartialEq)]
pub enum FxPass {
    /// SetRenderTarget (and the surface's viewport).
    Target(FxTarget),
    /// IDirect3DDevice8::Clear(D3DCLEAR_TARGET, colour).
    Clear { color: u32 },
    /// Strip of 4 vertices (top left, top right, bottom left, bottom right) on
    /// the scene texture. `flags`: 1 = stage 1 textured too; 2 = ONE / ONE blending in
    /// REVSUBTRACT, 4 = SRCALPHA / INVSRCALPHA, otherwise no blending; 8 = stage 0 SELECTARG1
    /// (texture, vertex alpha) then stage 1 SUBTRACT (current − texture), 0x10 = stage 1
    /// BLENDDIFFUSEALPHA (current, texture), otherwise stage 0 MODULATE (texture × vertex).
    Quad { verts: [FxVertex; 4], flags: u32 },
    /// Grids of the poison (stage 0 MODULATE2X) and of the water (MODULATE):
    /// indexed triangle list (texture × vertex), alpha MODULATE, without blending.
    Mesh { verts: Vec<FxVertex>, indices: Vec<u16>, modulate2x: bool },
    /// Strip of 4 vertices like `Quad`, stage 0 only on `tex`. `flags`: blending
    /// 1 = ONE / SRCALPHA, 0x40 = ONE / SRCALPHA in REVSUBTRACT, 2 = SRCCOLOR / ZERO,
    /// 0x10 = SRCCOLOR / ONE, 0x20 = ONE / ONE, 0x100 = ONE / ONE and pixel shader, otherwise
    /// no blending; stage 4 = DOTPRODUCT3 (texture, vertex), 8 = vertex colour and alpha,
    /// 0x80 = MODULATE2X (alpha MODULATE), otherwise MODULATE.
    Rect { verts: [FxVertex; 4], tex: FxTex, flags: u32 },
}

/// State of the effects of FD3DRenderInterface.
#[derive(Debug, Clone, Default)]
pub struct ScreenFx {
    /// +0xD3C … +0xD4C: poison (state, intensity, delay, maximum intensity, BGRA tint).
    pub poison_on: bool,
    pub poison: f32,
    pub poison_delay: f32,
    pub poison_max: f32,
    pub poison_hue: [u8; 4],
    /// +0xD50 … +0xD5C: blur (`blur_is_blur`) or sharpen, intensity, delay.
    pub blur_on: bool,
    pub blur_is_blur: bool,
    pub blur: f32,
    pub blur_delay: f32,
    /// +0xD60 … +0xD6C: injury (also underwater), elapsed time, intensity, delay.
    pub injured_on: bool,
    pub injured_time: f32,
    pub injured: f32,
    pub injured_delay: f32,
    /// Phase of the poison deformation.
    poison_phase: f32,
    /// Poison grid (starting positions, vertices, indices), built once.
    poison_grid: Option<(Vec<[f32; 2]>, Vec<FxVertex>, Vec<u16>)>,
    /// (Engine.dll): underwater window effect set by
    /// APlayerController::Tick.
    pub underwater: bool,
    /// +0xD34, +0xD38: the frame's water effect (bWaterEffectIsOn of the volume) and its intensity.
    pub water_on: bool,
    pub water: f32,
    /// Phase of the water deformation.
    water_phase: f32,
    /// 8 × 6 water grid, built once.
    water_grid: Option<(Vec<[f32; 2]>, Vec<FxVertex>, Vec<u16>)>,
    /// +0xD70 … +0xD9B: FlashEffectDesc of the view's zone.
    pub flash: FlashDesc,
    /// Bit 0 of +0xD70 while the frame's scene is drawn (vtable +0x90 before
    /// FCameraSceneNode::Render, cleared): then takes LightCurve
    /// 0xFF40 for the cartoon characters' ramp.
    pub cartoon_flash: bool,
}

/// ZoneInfo.FlashEffectStruct as stores it at +0xD70.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FlashDesc {
    /// +0xD70, bit 0: IsActivated (cleared); bit 1: NoGrey.
    pub activated: bool,
    pub no_grey: bool,
    /// +0xD74, +0xD75, +0xD76.
    pub contrast: u8,
    pub brightness: u8,
    pub layer_brightness: u8,
    /// +0xD78: LayerColor (B, G, R, A).
    pub layer_color: [u8; 4],
    /// +0xD7C: LayerSampling[8].
    pub sampling: [f32; 8],
}

/// Rounding to a 24-bit mantissa (FPU in single precision).
fn r(x: f64) -> f64 {
    x as f32 as f64
}

/// Rise or fall of an effect's intensity (same code): +dt / delay up to 1 if the effect is on, −dt / delay down to 0 otherwise.
fn ramp(on: bool, value: &mut f32, dt: f32, delay: f32) {
    if on {
        if *value < 1.0 {
            *value += dt / delay;
            if *value > 1.0 {
                *value = 1.0;
            }
        }
    } else if *value > 0.0 {
        *value -= dt / delay;
        if *value < 0.0 {
            *value = 0.0;
        }
    }
}

/// Nudge of the settings: an effect that is on
/// gains 0.0001 (clamped to 1) so that the scene goes into the texture from the next frame.
fn kick(value: &mut f32) {
    *value += RAMP_KICK;
    if 1.0 < *value {
        *value = 1.0;
    }
}

/// D3DCOLOR of the colours written as is in the code (0x80FFFFFF…).
const WHITE: u32 = 0xffff_ffff;

/// Vertices: quad (x1, −y1)-(x2, −y2); stage 0 on [inset0, 1 − inset0]²,
/// stage 1 on [inset1, 1 − inset1]², plus 1/1024 (half a texel of the 512 texture).
#[allow(clippy::too_many_arguments)]
fn quad(x1: f32, y1: f32, x2: f32, y2: f32, inset0: f32, inset1: f32, color: u32, flags: u32) -> FxPass {
    const HALF: f32 = 0.000_976_562_5;
    let (a, b) = (1.0 - inset0, 1.0 - inset1);
    let (s0, e0, s1, e1) = (inset0 + HALF, a + HALF, inset1 + HALF, b + HALF);
    let v = |x: f32, y: f32, u0: [f32; 2], u1: [f32; 2]| FxVertex { pos: [x, y], uv0: u0, uv1: u1, color };
    FxPass::Quad {
        verts: [
            v(x1, -y1, [s0, s0], [s1, s1]),
            v(x2, -y1, [e0, s0], [e1, s1]),
            v(x1, -y2, [s0, e0], [s1, e1]),
            v(x2, -y2, [e0, e0], [e1, e1]),
        ],
        flags,
    }
}

/// The whole scene copied back to the screen as it is (no blending): the first pass of the mods
/// build's heat shimmer when no effect of the game drew the scene back.
#[cfg(feature = "mods")]
pub(crate) fn copy_quad() -> FxPass {
    quad(-1.0, -1.0, 1.0, 1.0, 0.0, 0.0, WHITE, 0)
}

/// Grid of `w` × `h` vertices on the screen (x = 2i/(w−1) − 1, y = 2j/(h−1) − 1,
/// u = i/(w−1), v = 1 − j/(h−1)), index j·w + i, two triangles per cell.
fn grid(w: u32, h: u32, color: u32) -> (Vec<[f32; 2]>, Vec<FxVertex>, Vec<u16>) {
    let n = (w * h) as usize;
    let mut src = vec![[0.0f32; 2]; n];
    let mut verts = vec![FxVertex::default(); n];
    let fw = (w - 1) as f32;
    let fh = (h - 1) as f32;
    for i in 0..w {
        let u = i as f32 / fw;
        let x = (u + u) - 1.0;
        for j in 0..h {
            let t = j as f32 / fh;
            let y = (t + t) - 1.0;
            let k = (j * w + i) as usize;
            src[k] = [x, y];
            verts[k] = FxVertex { pos: [x, y], uv0: [u, 1.0 - t], uv1: [0.0; 2], color };
        }
    }
    let mut idx = Vec::new();
    for i in 0..w - 1 {
        for j in 0..h - 1 {
            let a = (j * w + i) as u16;
            let b = ((j + 1) * w + i) as u16;
            idx.extend([a, a + 1, b + 1, a, b + 1, b]);
        }
    }
    (src, verts, idx)
}

/// FColor (B, G, R, A) as D3DCOLOR (FColor::operator DWORD).
fn d3dcolor(c: [u8; 4]) -> u32 {
    (c[3] as u32) << 24 | (c[2] as u32) << 16 | (c[1] as u32) << 8 | c[0] as u32
}

/// Grey: `or reg, 0xFFFFFF00`, then twice `shl 8` and `or`.
fn grey(e: u32) -> u32 {
    ((((e | 0xffff_ff00) << 8) | e) << 8) | e
}

/// Vertices: (x1, −y1), (x2, −y1), (x1, −y2), (x2, −y2), coordinates
/// (u1, v1)-(u2, v2), colour on all four.
#[allow(clippy::too_many_arguments)]
fn rect(x1: f32, y1: f32, x2: f32, y2: f32, u1: f32, v1: f32, u2: f32, v2: f32, tex: FxTex, color: u32, flags: u32) -> FxPass {
    let v = |x: f32, y: f32, u: f32, w: f32| FxVertex { pos: [x, y], uv0: [u, w], uv1: [u, w], color };
    FxPass::Rect { verts: [v(x1, -y1, u1, v1), v(x2, -y1, u2, v1), v(x1, -y2, u1, v2), v(x2, -y2, u2, v2)], tex, flags }
}

/// LayerColor component × LayerBrightness / 255 (division through 0x80808081, truncated).
fn layer(c: u8, lb: u8) -> u32 {
    (c as i32 * lb as i32 / 255) as u32
}

impl crate::Vm {
    /// Screen effects of a drawn frame: UGameEngine::Draw's settings for the view of
    /// `controller` placed at `origin` (last PlayerCalcView), (scene into the
    /// texture if an intensity is not zero) then Apply2DEffect with GEngine->DeltaSeconds.
    /// Empty: scene on the screen.
    pub fn screen_fx_frame(&mut self, controller: Option<crate::ObjId>, origin: Option<[f32; 3]>) -> Vec<FxPass> {
        // Split screen (not in the original PC version): each view has its own state, advanced
        // once per frame by its own Apply2DEffect, instead of the device's single state drawn
        // (and advanced) once per viewport.
        if let (true, Some(pc)) = (self.split_screen_fx(), controller) {
            if !self.coop.screen_fx_drawn.contains(&pc) {
                self.coop.screen_fx_drawn.push(pc);
            }
            let mut own = std::mem::take(self.screen_fx_of(pc));
            std::mem::swap(&mut self.screen_fx, &mut own);
            let passes = self.screen_fx_view(controller, origin);
            std::mem::swap(&mut self.screen_fx, &mut own);
            self.screen_fx.cartoon_flash = own.cartoon_flash;
            *self.screen_fx_of(pc) = own;
            return passes;
        }
        self.screen_fx_view(controller, origin)
    }

    /// Split screen, once per level tick: the views not drawn since the last tick (the window
    /// shows one player's picture during a cinematic) still run their Apply2DEffect, so that
    /// their fades go on; otherwise a view hidden for a cinematic came back with the fade of
    /// the moment it was hidden (the wound effect of Plage00's start, turned off by the
    /// flashback, faded over the first second of the memory on the second player's screen).
    pub(crate) fn screen_fx_hidden_views(&mut self) {
        let drawn = std::mem::take(&mut self.coop.screen_fx_drawn);
        if !self.split_screen_fx() || drawn.is_empty() {
            return;
        }
        let dt = self.engine_delta;
        for (pc, fx) in &mut self.coop.screen_fx {
            if !drawn.contains(pc) && fx.redirected() {
                let _ = fx.apply_2d_effect(dt);
            }
        }
    }

    /// Split screen with several local players: one effects state per view.
    fn split_screen_fx(&self) -> bool {
        self.local_split() && self.coop.players.len() > 1
    }

    /// A LevelInfo effect native: the device's state, and every view's in split screen.
    pub(crate) fn screen_fx_all(&mut self, f: impl Fn(&mut ScreenFx)) {
        f(&mut self.screen_fx);
        for (_, fx) in &mut self.coop.screen_fx {
            f(fx);
        }
    }

    /// Effects state of `pc`'s view: the device's, or in split screen the view's own
    /// (a copy of the device's state the first time).
    pub(crate) fn screen_fx_of(&mut self, pc: crate::ObjId) -> &mut ScreenFx {
        if !self.split_screen_fx() {
            return &mut self.screen_fx;
        }
        let i = match self.coop.screen_fx.iter().position(|(p, _)| *p == pc) {
            Some(i) => i,
            None => {
                let fx = self.screen_fx.clone();
                self.coop.screen_fx.push((pc, fx));
                self.coop.screen_fx.len() - 1
            }
        };
        &mut self.coop.screen_fx[i].1
    }

    fn screen_fx_view(&mut self, controller: Option<crate::ObjId>, origin: Option<[f32; 3]>) -> Vec<FxPass> {
        self.view_effects(controller, origin);
        self.screen_fx.cartoon_flash = self.screen_fx.flash.activated;
        if !self.screen_fx.redirected() {
            // Mods build only: a script's heat shimmer, alone.
            #[cfg(feature = "mods")]
            return self.mod_heat_passes(controller, origin, false);
            #[cfg(not(feature = "mods"))]
            return Vec::new();
        }
        let dt = self.engine_delta;
        #[allow(unused_mut)]
        let mut passes = self.screen_fx.apply_2d_effect(dt);
        #[cfg(feature = "mods")]
        passes.extend(self.mod_heat_passes(controller, origin, true));
        passes
    }

    /// UGameEngine::Draw, after FCameraSceneNode: vtable +0x9C if the
    /// view's volume has bWaterEffectIsOn (+0x2CC, bit 0x40), with WaterEffectIntensity
    /// (+0x2E4); vtable +0x90 with the FlashEffectDesc (+0x22C) of the actor of the view's zone
    /// if it has IsActivated.
    fn view_effects(&mut self, controller: Option<crate::ObjId>, origin: Option<[f32; 3]>) {
        // FLevelSceneNode::Init (outside the editor): Viewport->Actor->ViewTarget, its
        // PhysicsVolume (+0xC8), or its HeadVolume (+0x2B0) if it is a pawn.
        let target = controller.and_then(|c| self.po(c, "ViewTarget"));
        let volume = target.and_then(|t| if self.is_a(t, "Pawn") { self.po(t, "HeadVolume") } else { self.native_object(t, crate::NativeField::PhysicsVolume) });
        if let Some(v) = volume {
            if self.pb(v, "bWaterEffectIsOn") {
                let intensity = self.pf(v, "WaterEffectIntensity");
                self.screen_fx.set_water(true, intensity);
            }
        }
        // Zone of the viewpoint (FCameraSceneNode +0x1D0), then ULevel::GetZoneActor
        // Zones[i].ZoneActor, otherwise the LevelInfo.
        let Some(origin) = origin else { return };
        let zone = self.level_bsp.as_ref().map_or(0, |m| m.point_region(origin).2 as usize);
        let Some(z) = self.get_zone_actor(zone) else { return };
        let Value::Struct(_, f) = self.prop(z, "FlashEffectDesc") else { return };
        let desc = flash_desc(&f);
        if desc.activated {
            self.screen_fx.set_flash(desc);
        }
    }
}

impl crate::Vm {
    /// ULevel::GetZoneActor: Model->Zones[zone].ZoneActor if the zone exists and has an actor, otherwise the LevelInfo
    /// (GetLevelInfo).
    pub(crate) fn get_zone_actor(&self, zone: usize) -> Option<crate::ObjId> {
        self.zone_actors.get(zone).copied().flatten().or(self.level)
    }
}

/// Fields of ZoneInfo.FlashEffectStruct in declaration order.
fn flash_desc(f: &[Value]) -> FlashDesc {
    let get = |i: usize| f.get(i).cloned().unwrap_or(Value::Void);
    let mut d = FlashDesc {
        activated: get(0).as_bool(),
        no_grey: get(1).as_bool(),
        contrast: get(2).as_byte(),
        brightness: get(3).as_byte(),
        layer_brightness: get(4).as_byte(),
        ..Default::default()
    };
    if let Value::Struct(_, c) = get(5) {
        for (k, v) in c.iter().take(4).enumerate() {
            d.layer_color[k] = v.as_byte();
        }
    }
    match get(6) {
        Value::Array(a) => {
            for (k, v) in a.iter().take(8).enumerate() {
                d.sampling[k] = v.as_float();
            }
        }
        _ => {
            for (k, s) in d.sampling.iter_mut().enumerate() {
                *s = get(6 + k).as_float();
            }
        }
    }
    d
}

impl ScreenFx {
    /// Vtable +0xA0: ALevelInfo::execSetPoisonEffect.
    pub fn set_poison(&mut self, on: bool, delay: f32, max: f32, hue: [u8; 4]) {
        self.poison_on = on;
        if !on {
            if delay == 0.0 {
                self.poison = 0.0;
            }
        } else {
            kick(&mut self.poison);
        }
        self.poison_delay = delay;
        self.poison_max = max;
        self.poison_hue = hue;
    }

    /// Vtable +0xA4: execSetBlurEffect (blur = true) / execSetSharpEffect, delay
    /// 0.5.
    pub fn set_blur(&mut self, on: bool, delay: f32, blur: bool) {
        self.blur_on = on;
        self.blur_is_blur = blur;
        if on {
            kick(&mut self.blur);
        }
        self.blur_delay = delay;
    }

    /// Vtable +0xA8: execSetInjuredEffect and the underwater window.
    pub fn set_injured(&mut self, on: bool, delay: f32) {
        self.injured_on = on;
        if !on {
            self.injured_delay = delay;
            if delay == 0.0 {
                self.injured = 0.0;
            }
        } else {
            kick(&mut self.injured);
            self.injured_delay = delay;
        }
    }

    /// Vtable +0x9C: the frame's water effect; when on, the scene goes into the
    /// texture.
    pub fn set_water(&mut self, on: bool, intensity: f32) {
        self.water_on = on;
        self.water = intensity;
    }

    /// Vtable +0x90: copy of the FlashEffectDesc; when activated, the scene
    /// goes into the texture.
    pub fn set_flash(&mut self, desc: FlashDesc) {
        self.flash = desc;
    }

    /// The scene is drawn into the 512 × 512 texture: by when
    /// an intensity is not zero, or by the frame's water and flash settings.
    pub fn redirected(&self) -> bool {
        self.poison != 0.0 || self.blur != 0.0 || self.injured != 0.0 || self.water_on || self.flash.activated
    }

    /// (vtable +0x94): flash, water, poison, blur, injury, each one if it is set or
    /// visible, with the frame duration `dt` (GEngine->DeltaSeconds).
    pub fn apply_2d_effect(&mut self, dt: f32) -> Vec<FxPass> {
        let mut out = Vec::new();
        if self.flash.activated {
            self.flash_pass(&mut out);
        }
        if self.water_on {
            self.water_pass(dt, &mut out);
        }
        if self.poison != 0.0 {
            self.poison_pass(dt, &mut out);
        }
        if self.blur != 0.0 {
            self.blur_pass(dt, &mut out);
        }
        if self.injured != 0.0 {
            self.injured_pass(dt, &mut out);
        }
        out
    }

    /// The scene (still in its texture) is veiled with grey, then copied back to
    /// the screen in grey levels (DOTPRODUCT3); with NoGrey, darkened then copied in
    /// MODULATE2X. If LayerBrightness is not zero, each LayerSampling pair (up to the
    /// first one below 0.1) shrinks the scene into the 128 × 128 texture and adds it enlarged to
    /// the screen, tinted with LayerColor × LayerBrightness / 255.
    fn flash_pass(&mut self, out: &mut Vec<FxPass>) {
        self.flash.activated = false;
        let d = self.flash;
        let (contrast, brightness, lb) = (d.contrast as u32, d.brightness as i32, d.layer_brightness);
        let [b, g, r_, _] = d.layer_color;
        let tint = (((layer(r_, lb) | 0xff00) << 8 | layer(g, lb)) << 8) | layer(b, lb);
        // LayerSampling pairs at 0.1 at least (fcomp: a NaN value stops too).
        let pairs = (0..4).map(|i| (d.sampling[2 * i], d.sampling[2 * i + 1])).take_while(|&(s0, s1)| s0 >= 0.1 && s1 >= 0.1);
        if !d.no_grey {
            let c = 0xc0 - (contrast >> 1);
            let veil = ((contrast << 8 | c) << 8 | c) << 8 | c;
            let copy = grey((brightness as u32 >> 1) + 0x80);
            out.push(rect(-1.0, -1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, FxTex::None, veil, 9));
            out.push(FxPass::Target(FxTarget::Back));
            out.push(rect(-1.0, -1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, FxTex::Scene, copy, 4));
            if lb != 0 {
                for (s0, s1) in pairs {
                    // 0.25 / (128 × s), rounded to 24 bits; the shrunk image overflows by 2a, 2b.
                    let a = 0.25 / (128.0 * s0);
                    let b = 0.25 / (128.0 * s1);
                    let (a2, b2) = (a + a, b + b);
                    out.push(FxPass::Target(FxTarget::Small));
                    out.push(rect(-1.0, -1.0, (s0 + s0 - 1.0) + a2, (s1 + s1 - 1.0) + b2, 0.0, 0.0, 1.0, 1.0, FxTex::Scene, copy, 4));
                    out.push(FxPass::Target(FxTarget::Back));
                    out.push(rect(-1.0, -1.0, 1.0, 1.0, a, b, a2 + s0, b2 + s1, FxTex::Small, tint, 0x20));
                }
            }
        } else {
            let k = (contrast * 255) as i32 / (contrast * 2 + 0x1fe) as i32;
            let bright = (brightness - 0x80) / 2 + (contrast >> 2) as i32 + 0x40;
            let bright = grey(bright as u32);
            out.push(rect(-1.0, -1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, FxTex::None, grey(k as u32), 0x48));
            out.push(FxPass::Target(FxTarget::Back));
            out.push(rect(-1.0, -1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, FxTex::Scene, bright, 0x80));
            if lb != 0 {
                for (s0, s1) in pairs {
                    let (h0, h1) = (s0 * 0.5, s1 * 0.5);
                    out.push(FxPass::Target(FxTarget::Small));
                    out.push(rect(-s0, -s1, s0, s1, 0.0, 0.0, 1.0, 1.0, FxTex::Scene, bright, 0x80));
                    out.push(FxPass::Target(FxTarget::Back));
                    out.push(rect(-1.0, -1.0, 1.0, 1.0, 0.5 - h0, 0.5 - h1, 0.5 + h0, 0.5 + h1, FxTex::Small, tint, 0x20));
                }
            }
        }
    }

    /// 8 × 6 grid whose inner vertices wave,
    /// drawn to the screen on the scene texture, stage 0 MODULATE.
    fn water_pass(&mut self, dt: f32, out: &mut Vec<FxPass>) {
        self.water_on = false;
        if self.water_grid.is_none() {
            self.water_grid = Some(grid(8, 6, WHITE));
        }
        self.water_phase = dt * 3.0 + self.water_phase;
        if 6.283_185_5 <= self.water_phase {
            self.water_phase -= 6.283_185_5;
        }
        let amp = self.water * 10.0 * 0.001_562_5;
        let phase = self.water_phase;
        let (src, verts, idx) = self.water_grid.as_mut().unwrap();
        // Columns 1 to 6, rows 1 to 4; appSin and appCos in double of arguments
        // rounded to 24 bits, their result kept on the stack and multiplied by the amplitude
        // (FMUL dword, a single 24-bit rounding).
        for i in 1..7u32 {
            for j in 1..5u32 {
                let k = (j * 8 + i) as usize;
                let [x, y] = src[k];
                let sx = crate::ai_move::fmul_d(amp, ((x * 5.0 + phase) as f64).dsin());
                let cy = crate::ai_move::fmul_d(amp, ((y * 5.0 + phase) as f64).dcos());
                verts[k].pos = [sx + x, cy + y];
            }
        }
        out.push(FxPass::Target(FxTarget::Back));
        out.push(FxPass::Mesh { verts: verts.clone(), indices: idx.clone(), modulate2x: false });
    }

    /// Deformed and tinted 20 × 15 grid, drawn on the screen.
    fn poison_pass(&mut self, dt: f32, out: &mut Vec<FxPass>) {
        if self.poison_grid.is_none() {
            self.poison_grid = Some(grid(20, 15, WHITE));
        }
        ramp(self.poison_on, &mut self.poison, dt, self.poison_delay);
        self.poison_phase = dt * 5.0 + self.poison_phase;
        if 94.247_78 <= self.poison_phase {
            self.poison_phase -= 94.247_78;
        }
        let (intensity, phase, hue, max) = (self.poison, self.poison_phase, self.poison_hue, self.poison_max);
        let (src, verts, idx) = self.poison_grid.as_mut().unwrap();
        out.push(FxPass::Target(FxTarget::Back));
        // Tint = component × I + (1 − I) × 128 (truncated), alpha 255.
        let k = r((1.0 - intensity as f64) * 128.0);
        let ch = |c: u8| r(r(c as f64 * intensity as f64) + k) as i32 as u8;
        let color = d3dcolor([ch(hue[0]), ch(hue[1]), ch(hue[2]), 0xff]);
        let im = intensity * max;
        let a = phase * 0.2;
        let b = phase * 0.5;
        for i in 1..19u32 {
            for j in 1..14u32 {
                let kk = (j * 20 + i) as usize;
                let [x, y] = src[kk];
                let d = (x * x + y * y).sqrt();
                // fsqrt rounded to 24 bits, then × 10 and + a; the cosine of the angle stays on the
                // stack in extended precision, the sine is stored as a float.
                let e = r(r(r((d as f64).sqrt()) * 10.0) + a as f64);
                let ang = r(r(e.dcos() * im as f64) * 0.234375);
                let (s, c) = (ang.dsin() as f32, ang.dcos());
                let mut px = r(r(x as f64 * c) - (y * s) as f64) as f32;
                let mut py = r(r(y as f64 * c) + (x * s) as f64) as f32;
                let (sb, cb) = ((b as f64).dsin() as f32, (b as f64).dcos());
                let w = (1.42 - d) * 0.039_062_5;
                px = sb * w + px;
                py = r(w as f64 * cb) as f32 + py;
                verts[kk].pos = [px, py];
                verts[kk].color = color;
            }
        }
        for i in 0..20u32 {
            verts[i as usize].color = color;
            verts[(14 * 20 + i) as usize].color = color;
        }
        for j in 1..14u32 {
            verts[(j * 20) as usize].color = color;
            verts[(j * 20 + 19) as usize].color = color;
        }
        out.push(FxPass::Mesh { verts: verts.clone(), indices: idx.clone(), modulate2x: true });
    }

    /// Screen cleared to black, then blur (two enlarged quads, blended at 50 %) or
    /// sharpen (copy, then subtraction of the difference with an enlarged image).
    fn blur_pass(&mut self, dt: f32, out: &mut Vec<FxPass>) {
        ramp(self.blur_on, &mut self.blur, dt, self.blur_delay);
        out.push(FxPass::Target(FxTarget::Back));
        out.push(FxPass::Clear { color: 0 });
        if self.blur_is_blur {
            let s = self.blur * 0.0078125;
            out.push(quad(-1.0, -1.0, 1.0, 1.0, s * 3.0, r(s as f64 * 3.5) as f32, 0x80ff_ffff, 0x11));
            out.push(quad(-1.0, -1.0, 1.0, 1.0, r(s as f64 * 4.0) as f32, r(s as f64 * 4.5) as f32, 0x80ff_ffff, 0x15));
        } else {
            let s = self.blur * 0.003_906_25;
            out.push(quad(-1.0, -1.0, 1.0, 1.0, 0.0, 0.0, WHITE, 0));
            out.push(quad(-1.0, -1.0, 1.0, 1.0, 0.0, s, WHITE, 0xb));
            out.push(quad(-1.0, -1.0, 1.0, 1.0, 0.0, s + s, WHITE, 0xb));
        }
    }

    /// Background cleared to black, then two quads of the image that wave (sine and cosine of the elapsed time),
    /// alpha 0x55, the second blended on top.
    fn injured_pass(&mut self, dt: f32, out: &mut Vec<FxPass>) {
        self.injured_time = dt + self.injured_time;
        ramp(self.injured_on, &mut self.injured, dt, self.injured_delay);
        let (t, i) = (self.injured_time as f64, self.injured as f64);
        // Double arguments from values rounded to 24 bits; results rounded at each
        // operation.
        let wave = |v: f64, k: f64, sign: f64| r(r(r(r(v + sign) * i) * k) + sign) as f32;
        let inset1 = r(r(r(r(t * 0.3) + 0.4).dsin() * i) * 0.03) as f32;
        let inset0 = r(r(t.dcos() * i) * 0.025) as f32;
        let y2 = wave(r(r(t * 0.8) + 1.2).dcos(), 0.0225, 1.0);
        let x2 = wave(r(r(t * 0.8) + 1.7).dcos(), 0.035, 1.0);
        let y1 = wave(r(r(t * 0.4) + 1.0).dcos(), 0.015, -1.0);
        let x1 = wave(r(t * 0.8).dcos(), 0.025, -1.0);
        out.push(FxPass::Target(FxTarget::Back));
        // Clear(D3DCLEAR_TARGET, 0): the border that the first quad does not cover stays black.
        out.push(FxPass::Clear { color: 0 });
        out.push(quad(x1, y1, x2, y2, inset0, inset1, 0x55ff_ffff, 0x11));
        let inset1 = r(r(r(t * 0.7).dsin() * i) * 0.0225) as f32;
        let inset0 = r(r(r(r(t * 1.1) + 0.5).dcos() * i) * 0.02) as f32;
        out.push(quad(-1.0, -1.0, 1.0, 1.0, inset0, inset1, 0x55ff_ffff, 0x15));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default values of ZoneInfo.FlashEffectDesc (ZoneInfo.uc) activated.
    fn zone_default() -> FlashDesc {
        FlashDesc {
            activated: true,
            no_grey: false,
            contrast: 128,
            brightness: 64,
            layer_brightness: 128,
            layer_color: [255, 204, 204, 255],
            sampling: [0.4, 0.4, 0.55, 0.55, 0.75, 0.75, 0.0, 0.0],
        }
    }

    #[test]
    fn flash_grey_and_layers() {
        let mut fx = ScreenFx::default();
        fx.set_flash(zone_default());
        assert!(fx.redirected());
        let passes = fx.apply_2d_effect(0.0);
        assert!(!fx.flash.activated);
        // Veil, grey copy, then three layers (the fourth pair is below 0.1).
        assert_eq!(passes.len(), 3 + 3 * 4);
        let FxPass::Rect { verts, tex: FxTex::None, flags: 9 } = &passes[0] else { panic!("{:?}", passes[0]) };
        // Contrast 128: alpha 0x80, grey 0xC0 − 64 = 0x80.
        assert_eq!(verts[0].color, 0x8080_8080);
        assert_eq!(verts[0].pos, [-1.0, 1.0]);
        assert_eq!(passes[1], FxPass::Target(FxTarget::Back));
        let FxPass::Rect { verts, tex: FxTex::Scene, flags: 4 } = &passes[2] else { panic!() };
        // Brightness 64: 32 + 0x80 = 0xA0.
        assert_eq!(verts[0].color, 0xffa0_a0a0);
        let FxPass::Rect { verts, tex: FxTex::Scene, flags: 4 } = &passes[4] else { panic!() };
        let a = 0.25 / (128.0 * 0.4f32);
        assert_eq!(verts[3].pos, [(0.4 + 0.4 - 1.0) + (a + a), -((0.4 + 0.4 - 1.0) + (a + a))]);
        let FxPass::Rect { verts, tex: FxTex::Small, flags: 0x20 } = &passes[6] else { panic!() };
        // LayerColor (B 255, G 204, R 204) × 128 / 255.
        assert_eq!(verts[0].color, 0xff66_66_80);
        assert_eq!(verts[0].uv0, [a, a]);
        assert_eq!(verts[3].uv0, [(a + a) + 0.4, (a + a) + 0.4]);
        // Next frame without a setting: nothing any more.
        assert!(!fx.redirected());
    }

    #[test]
    fn flash_without_grey() {
        let mut fx = ScreenFx::default();
        // GenFlash.uc: Contrast 0, Brightness 255, NoGrey, layers 1.0.
        fx.set_flash(FlashDesc {
            activated: true,
            no_grey: true,
            contrast: 0,
            brightness: 255,
            layer_brightness: 255,
            layer_color: [192, 192, 192, 255],
            sampling: [1.0, 0.4, 0.55, 1.0, 0.75, 0.75, 1.0, 1.0],
        });
        let passes = fx.apply_2d_effect(0.0);
        let FxPass::Rect { verts, flags: 0x48, .. } = &passes[0] else { panic!() };
        assert_eq!(verts[0].color, 0xff00_0000);
        let FxPass::Rect { verts, flags: 0x80, .. } = &passes[2] else { panic!() };
        // (255 − 128) / 2 + 0 + 0x40 = 0x7F.
        assert_eq!(verts[0].color, 0xff7f_7f7f);
        assert_eq!(passes.len(), 3 + 4 * 4);
        let FxPass::Rect { verts, tex: FxTex::Small, .. } = &passes[6] else { panic!() };
        assert_eq!(verts[0].uv0, [0.0, 0.3]);
    }

    #[test]
    fn water_grid_test() {
        let mut fx = ScreenFx::default();
        fx.set_water(true, 0.5);
        assert!(fx.redirected());
        let passes = fx.apply_2d_effect(0.1);
        assert!(!fx.water_on);
        let FxPass::Mesh { verts, indices, modulate2x: false } = &passes[1] else { panic!() };
        assert_eq!((verts.len(), indices.len()), (48, 70 * 3));
        // Edges fixed, interior moved.
        assert_eq!(verts[0].pos, [-1.0, -1.0]);
        let [x, y] = [2.0 / 7.0 - 1.0, 2.0 / 5.0 - 1.0];
        let amp = 0.5f32 * 10.0 * 0.001_562_5;
        let phase = 0.1f32 * 3.0;
        assert_eq!(verts[9].pos, [((x * 5.0 + phase) as f64).dsin() as f32 * amp + x, ((y * 5.0 + phase) as f64).dcos() as f32 * amp + y]);
    }
}

