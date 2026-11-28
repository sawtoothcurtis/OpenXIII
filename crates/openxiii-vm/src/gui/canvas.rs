//! Canvas: what the HUD and menu scripts draw every frame, reimplemented from UCanvas
//! (Engine.dll). The natives fill, like FCanvasUtil (DrawTile
//! DrawTriangle, DrawLine), a list of primitives in screen pixels that the renderer
//! draws over the scene with each one's material.
//!
//! UCanvas::Update, DealWithResetEvent, DrawTile (styles,
//! bDialogWindow, bUseBorder), (text of a font, page by page), WrappedPrint,
//! ClippedStrLen / ClippedPrint, DrawTextJustified,
//! WrapStringToArray, DrawTileStretched / Scaled / Bound /
//! ScaleBound / Justified, execDrawMsgboxBackground,
//! DrawFrame and the class's exec* natives. Computations in single precision in
//! machine-code order.
//!
//! FRenderInterface +0xAC (FD3DRenderInterface: D3DTSS_MAG/MINFILTER of stage 0
//! as D3DTEXF_POINT for 0, LINEAR for 1), called with 0 then 1 around WrappedPrint and
//! execDrawTextClipped: the primitives drawn between the two are marked `point`. The
//! primitives of an FCanvasUtil batch left open before the text would be flushed with this
//! filter; UCanvas::DrawTile, DrawStdBackGround, DrawPlayerInfo… flush their batch on exit,
//! this case is not tracked. The "%c" of
//! WrapStringToArray receives the address of the text instead of the character in the game (low
//! byte of a pointer): we put the character there.

// Operands in machine-code order (u = t + u…).
#![allow(clippy::assign_op_pattern)]

use crate::value::Value;
use crate::{ObjId, Vm};

/// FCanvasUtil primitive: vertices in screen pixels (z = 0), texture coordinates already
/// divided by MaterialUSize / MaterialVSize (without a material: as is), FColor colour
/// (B, G, R, A). `material`: path of a material, "#n" for the VM's render texture n,
/// "#white" for GEngine->WhiteTextureMaterial, "#canvas:fb:test:ref|material" for the
/// UFinalBlend of UCanvas::DrawTile, "#canvasline:fb:test" for that of FCanvasUtil::DrawLine
/// (on WhiteTextureMaterial), empty without a material.
#[derive(Debug, Clone)]
pub enum HudDraw {
    /// DrawTile: two triangles (x1, y1)-(x2, y1)-(x2, y2) and (x1, y1)-(x2, y2)-(x1, y2);
    /// `point`: stage 0 with D3DTEXF_POINT filtering (text, FRenderInterface +0xAC).
    /// `enlarged`: drawn larger than its own size by a menu, page or HUD scale
    /// (gui::place_scaled_xy); with the improved build's enhanced graphics a text tile so
    /// marked is read with the bicubic filter instead (gui::reads_bicubic).
    /// `glyph`: a font character cell, set where every letter is emitted (draw_string, free_text)
    /// whatever its filtering: ClippedPrint and DrawTextJustified draw text without POINT, so
    /// `point` alone does not tell text apart.
    Tile { x1: f32, y1: f32, x2: f32, y2: f32, uv: [f32; 4], material: std::sync::Arc<str>, color: [u8; 4], point: bool, enlarged: bool, glyph: bool },
    /// DrawTriangle: vertices p[0], p[1], p[2] at coordinates (u1, v1), (u2, v1), (u2, v2).
    Triangle { p: [[f32; 2]; 3], uv: [f32; 4], material: std::sync::Arc<str>, color: [u8; 4] },
    /// DrawLine (PT_LineList).
    Line { from: [f32; 2], to: [f32; 2], material: std::sync::Arc<str>, color: [u8; 4] },
    /// UCanvas::execDrawPortal: the scene seen from `location`, `rotation` (horizontal field `fov`)
    /// in the rectangle (x, y, width, height); the following primitives go over it.
    /// `filter`: colour (B, G, R, A) of the FB_Modulate DrawTile that the portal's FCameraSceneNode::Render
    /// puts on its rectangle (the player's colour filter, see `Vm::camera_filter`).
    Portal { rect: [f32; 4], location: [f32; 3], rotation: [i32; 3], fov: f32, clear_z: bool, filter: Option<[u8; 4]> },
}

/// UCanvas state kept outside the script: materials read (sizes), static UFinalBlend of
/// DrawTile, frame duration (GEngine->FrameLenght).
#[derive(Default)]
pub struct CanvasState {
    pub(crate) materials: openxiii_assets::material::Materials,
    /// FrameBufferBlending of the static UFinalBlend: styles ≥ 6 keep it.
    pub(crate) blend: u8,
    pub frame_length: f32,
    /// Stage 0 filtering set by FRenderInterface +0xAC (true: D3DTEXF_POINT).
    pub(crate) point_filter: bool,
    /// MaterialUSize / VSize already read per package path: the properties of the package's materials
    /// do not change (only their animation state moves), the answer is the same.
    sizes_list: std::collections::HashMap<String, (i32, i32)>,
    /// Material keys of the primitives, shared (a letter does not copy the name again).
    names_list: std::collections::HashSet<std::sync::Arc<str>>,
}

impl std::fmt::Debug for CanvasState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanvasState").field("blend", &self.blend).field("frame_length", &self.frame_length).finish()
    }
}

/// Script call to the audio subsystem, with the arguments that Engine.dll's natives
/// pass to UHXAUDIOSubsystem (AActor::execPlaySound…).
#[derive(Debug, Clone, PartialEq)]
pub enum AudioCall {
    /// PlaySound / PlayVoice / PlayMenu / PlayMusic / PlayStrAmb / PlayRolloffSound /
    /// PlayFiringSound: actor, attenuation actor, sound, category, parameters Param1…5
    /// (−1 = missing). `check_handler`: without a handler (byte +0x29 of the sound), +0xA0 even with
    /// parameters (all except PlayMenu). `first_free`: Param1 at −1 also chooses +0xA0 (all
    /// except PlayFiringSound, which only looks at the handler).
    Play {
        actor: Option<ObjId>,
        rolloff: Option<ObjId>,
        sound: String,
        slot: i32,
        params: Vec<i32>,
        check_handler: bool,
        first_free: bool,
    },
    /// StopSound / StopMusic / StopActorSounds…: category mask, notify the script.
    Stop {
        actor: Option<ObjId>,
        sound: Option<String>,
        kinds: i32,
        notify: bool,
    },
    XiiiStep {
        actor: ObjId,
        step: String,
        speed: f32,
        end_step: i32,
        silent: bool,
    },
    NPCStep {
        actor: ObjId,
        step: String,
        speed: f32,
        category: i32,
        silent: bool,
    },
    /// PlaySndPNJOno / PlaySndDeathOno.
    Ono {
        actor: ObjId,
        ono: String,
        code_mesh: i32,
        timbre: i32,
        death: bool,
    },
    /// PauseSound / ResumeSound (PauseMusic, PauseAllSounds…): category mask.
    Pause {
        kinds: i32,
    },
    Resume {
        kinds: i32,
    },
    /// ALevelInfo::execIncAttente…: counter MusicVars[index] changed by `delta`.
    MusicCounter {
        index: usize,
        delta: i32,
        /// MusicVars[index] (name, type) and its value after the change, MusicVars(1).Value: read by
        /// the native at the time of the call (the counter changes even without an audio subsystem).
        name: String,
        ty: u8,
        value: i32,
        alert: i32,
    },
    /// SetVolume: UHXAUDIOSubsystem (vtable +0xB8) for the categories `kinds`.
    Volume {
        kinds: i32,
        volume: f32,
    },
    /// SetMusicSliderPos (UHXAUDIOSubsystem::SetMusicPosition).
    MusicPosition {
        pos: i32,
    },
    StrVoice {
        actor: ObjId,
        name: String,
        rolloff: Option<ObjId>,
    },
}

pub(crate) fn color_of(v: &Value) -> [u8; 4] {
    // Order of Color's fields in XIII: B, G, R, A.
    match v {
        Value::Struct(_, f) if f.len() == 4 => [f[2].as_byte(), f[1].as_byte(), f[0].as_byte(), f[3].as_byte()],
        _ => [255; 4],
    }
}

/// Camera view requested by the script for a render texture (RenderTargetMaterial.Update,
/// URenderTargetMaterial::Update, which stores the last four fields in the
/// viewport's PlayerController for the duration of the render: +0x5B4, +0x5B8, +0x5B1 and +0x5BC).
#[derive(Debug, Clone)]
pub struct RenderView {
    /// Rectangle of the texture (x, y, width, height) where the view is rendered.
    pub rect: [i32; 4],
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    pub fov: f32,
    /// FilterColor (R, G, B, A) and HighLight passed to URenderTargetMaterial::Update.
    pub filter: [u8; 4],
    /// |HighLight| (+0x5B8).
    pub highlight: f32,
    /// HighLight > 0 (+0x5B1): brightens the frame.
    pub high_light: bool,
    /// FilterTexture (+0x5BC): draws this texture (if it is a UTexture), tinted
    /// by FilterColor, instead of WhiteTexture; path of the material, empty otherwise.
    pub filter_texture: String,
    /// Number of the call to Update (increasing): a new call renders the view again.
    pub serial: u64,
    /// DistanceFogColor (R, G, B, A) of CamLocation's zone if it has bForceClear.
    pub force_clear: Option<[u8; 4]>,
}

/// Path of the material, or "#n" for an object created by the script (render texture n).
pub(crate) fn material_path(v: &Value) -> String {
    match v {
        Value::Asset(p) => p.clone(),
        Value::Object(Some(o)) => format!("#{o}"),
        _ => String::new(),
    }
}

/// 1/255 of FColor::operator FPlane.
const INV_255: f32 = f32::from_bits(0x3B80_8081);

/// FISTP with round to nearest (even).
fn fistp(x: f32) -> i32 {
    x.round_ties_even() as i32
}

/// Truncated float → integer conversion.
fn ftol(x: f32) -> i32 {
    x as i32
}

/// FColor::operator FPlane: colour (B, G, R, A) → plane (R, G, B, A) / 255.
pub(crate) fn fplane(c: [u8; 4]) -> [f32; 4] {
    [c[2] as f32 * INV_255, c[1] as f32 * INV_255, c[0] as f32 * INV_255, c[3] as f32 * INV_255]
}

/// FColor::FColor(FPlane): fistp(X × 255 − 0.5) clamped to [0, 255].
pub(crate) fn fcolor(p: [f32; 4]) -> [u8; 4] {
    let ch = |v: f32| fistp(v * 255.0 - 0.5).clamp(0, 255) as u8;
    [ch(p[2]), ch(p[1]), ch(p[0]), ch(p[3])]
}

/// Script text as ANSI bytes (the game is not compiled as Unicode).
pub(crate) fn ansi(s: &str) -> Vec<u8> {
    s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()
}

fn from_ansi(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// Material of GEngine->WhiteTextureMaterial (UProxyBitmapMaterial of a white texture,
/// UEngine::U).
pub const WHITE: &str = "#white";

pub(crate) fn color_value(c: [u8; 4]) -> Value {
    Value::Struct("object.color".into(), c.iter().map(|&b| Value::Byte(b)).collect())
}

/// Script Color in the order of its fields, that of an FColor: B, G, R, A.
pub(crate) fn bgra(v: &Value) -> [u8; 4] {
    match v {
        Value::Struct(_, f) if f.len() == 4 => [f[0].as_byte(), f[1].as_byte(), f[2].as_byte(), f[3].as_byte()],
        _ => [0; 4],
    }
}

/// UFont: size of a character, USize and VSize of its cell.
fn char_size(f: &openxiii_assets::font::Font, ch: u8) -> (i32, i32) {
    let cpp = f.characters_per_page.max(1);
    let page = ch as usize / cpp;
    let Some(p) = f.pages.get(page) else { return (0, 0) };
    match p.glyphs.get(ch as usize - cpp * page) {
        Some(g) => (g.width, g.height),
        None => (0, 0),
    }
}

/// Text placement of UCanvas::DrawTextJustified: CurY = h / 2 − YL / 2
/// (integer division), CurX = 0 (left, or centred when the text is wider than the
/// rectangle), centred w / 2 − XL / 2, or right w − XL; another justification leaves CurX at 0.
pub(crate) fn place_justified(xl: i32, yl: i32, justification: u8, x1: f32, y1: f32, x2: f32, y2: f32) -> (f32, f32) {
    let cy = (y2 - y1) * 0.5 - (yl / 2) as f32;
    let cx = match justification {
        1 => {
            let w = x2 - x1;
            if xl as f32 <= w {
                w * 0.5 - (xl / 2) as f32
            } else {
                0.0
            }
        }
        2 => (x2 - x1) - xl as f32,
        _ => 0.0,
    };
    (cx, cy)
}

/// UCanvas::ClippedStrLen without the canvas: sum of ftol((USize + SpaceX) × ScaleX),
/// without SpaceX after the last character, and the largest ftol(VSize × ScaleY).
pub(crate) fn clipped_str_len(f: &openxiii_assets::font::Font, space_x: f32, scale_x: f32, scale_y: f32, text: &[u8]) -> (i32, i32) {
    let (mut xl, mut yl) = (0i32, 0i32);
    for (k, &ch) in text.iter().enumerate() {
        if ch == 0 {
            break;
        }
        let (w, h) = char_size(f, ch);
        let mut fw = w as f32;
        if text.get(k + 1).is_some_and(|&n| n != 0) {
            fw += space_x;
        }
        let cw = ftol(fw * scale_x);
        let chh = ftol(h as f32 * scale_y);
        xl += cw;
        if yl < chh {
            yl = chh;
        }
    }
    (xl, yl)
}

/// Lines of UCanvas::WrappedPrint: (start, length, width, height) of each.
/// The pen starts at CurX on the first line, at 0 afterwards (CurX reset to 0 after each line); spaces and returns skipped between two lines; stops when nothing fits.
pub(crate) fn wrapped_lines(f: &openxiii_assets::font::Font, space_x: f32, space_y: f32, clip_x: f32, cur_x: f32, text: &[u8]) -> Vec<(usize, usize, i32, i32)> {
    let at = |k: usize| text.get(k).copied().unwrap_or(0);
    let mut out = Vec::new();
    let mut t = 0usize;
    let mut pen = ftol(cur_x);
    loop {
        let (last, line_w, line_h) = cut_line(f, pen, space_x, space_y, clip_x, &text[t.min(text.len())..]);
        if last == 0 {
            break;
        }
        out.push((t, last, line_w, line_h));
        pen = 0;
        t += last;
        while at(t) == b' ' || at(t) == b'\n' {
            t += 1;
        }
        if at(t) == 0 {
            break;
        }
    }
    out
}

/// One line of UCanvas::WrappedPrint: from pen `pen` (CurX
/// rounded), each character advances by ftol(USize + SpaceX); the height is ftol(SpaceY) + VSize,
/// the largest of the line. The line stops at the first character that goes past ClipX; it is
/// cut after the last whole word (followed by a space, a return or the end), or, as long
/// as no word is whole, after the last character that fits. Returns (length, width,
/// height), length 0 if nothing fits.
pub(crate) fn cut_line(f: &openxiii_assets::font::Font, mut pen: i32, space_x: f32, space_y: f32, clip_x: f32, text: &[u8]) -> (usize, i32, i32) {
    let at = |k: usize| text.get(k).copied().unwrap_or(0);
    let (mut last, mut line_w, mut line_h, mut max_h) = (0usize, 0i32, 0i32, 0i32);
    let mut got = false;
    let mut i = 0usize;
    loop {
        let ch = at(i);
        if ch == 0 || ch == b'\n' {
            break;
        }
        let (w, h) = char_size(f, ch);
        pen += ftol(w as f32 + space_x);
        let hh = ftol(space_y) + h;
        if max_h < hh {
            max_h = hh;
        }
        if pen as f32 > clip_x {
            break;
        }
        let next = at(i + 1);
        i += 1;
        let brk = next == b' ' || next == b'\n' || next == 0;
        if brk || !got {
            last = i;
            line_w = pen;
            line_h = max_h;
            if brk {
                got = true;
            }
        }
    }
    (last, line_w, line_h)
}

/// FCameraSceneNode::Render in progress between `hud_begin` and `hud_end`.
#[derive(Debug)]
pub struct HudFrame {
    canvas: ObjId,
    players: Vec<ObjId>,
    precaching: bool,
    saved_action: Option<Value>,
}

impl Vm {
    pub(crate) fn cf(&self, c: ObjId, name: &str) -> f32 {
        self.propr(c, name).as_float()
    }

    pub(crate) fn set_cf(&mut self, c: ObjId, name: &str, v: f32) {
        self.set_prop(c, name, Value::Float(v));
    }

    /// OpenXIII addition, without an equivalent in the original: centred 4:3 frame for the comic panels.
    /// HudCartoonWindow.DrawWnd (script) places its panels in 640 × 480 coordinates multiplied by
    /// ClipX / 640, without looking at ClipY: the original game only has 4:3 modes
    /// (UD3DRenderDevice::GetAvailableRes, see `available_res`), where ClipY is
    /// always ClipX × 3 / 4. In a wider window, the bottom of the panels went off the screen
    /// ("ALARM!" panel of SPADS01, Alarme2 / Alarme3 / GameOver, Y = 264 and 128 high).
    /// If `f` is this DrawWnd, the canvas is wider than 4:3 and panel `this` would go off the
    /// bottom, its XL / YL / DXS / DYS are changed for the duration of the call so that it is drawn
    /// as in a centred 4:3 frame. Only this panel moves: the other panels of the chain
    /// (NextHudWnd.DrawWnd, called inside) and the dialogue portrait, stuck to the text of
    /// HudDlg which keeps the ClipX / 640 scale, stay in place.
    /// Returns the panel and its previous fields, to be restored after the call.
    pub(crate) fn cases_bd_frame_43(&mut self, f: crate::types::FuncId, this: ObjId, args: &[Option<Value>]) -> Option<(ObjId, [f32; 4])> {
        let d = &self.functions[f];
        if d.name != "DrawWnd" || !self.classes[d.owner].name.eq_ignore_ascii_case("HudCartoonWindow") {
            return None;
        }
        let c = args.first()?.as_ref()?.as_object()?;
        let hud = self.propr(this, "Owner").as_object()?;
        let fields_list = ["XL", "YL", "DXS", "DYS"].map(|n| self.cf(this, n));
        let margins = (self.cf(hud, "LeftMargin"), self.cf(hud, "UpMargin"));
        let new_value = case_frame_43(fields_list, margins, self.cf(c, "ClipX"), self.cf(c, "ClipY"))?;
        for (n, v) in ["XL", "YL", "DXS", "DYS"].into_iter().zip(new_value) {
            self.set_cf(this, n, v);
        }
        Some((this, fields_list))
    }

    /// Restores XL / YL / DXS / DYS after `cases_bd_frame_43`.
    pub(crate) fn cases_bd_frame_43_end(&mut self, w: ObjId, fields_list: [f32; 4]) {
        if self.objects.get(w).is_some_and(|o| o.is_some()) {
            for (n, v) in ["XL", "YL", "DXS", "DYS"].into_iter().zip(fields_list) {
                self.set_cf(w, n, v);
            }
        }
    }

    fn draw_color(&self, c: ObjId) -> [u8; 4] {
        bgra(self.propr(c, "DrawColor"))
    }

    fn canvas_style(&self, c: ObjId) -> u8 {
        self.propr(c, "Style").as_byte()
    }

    pub(crate) fn font_path(v: &Value) -> Option<String> {
        match v {
            Value::Asset(p) if !p.is_empty() => Some(p.clone()),
            _ => None,
        }
    }

    fn canvas_font(&self, c: ObjId) -> Option<String> {
        Self::font_path(self.propr(c, "Font"))
    }

    /// Game font (read once from its package).
    pub fn font(&mut self, path: &str) -> Option<std::rc::Rc<openxiii_assets::font::Font>> {
        if let Some(f) = self.fonts.get(path) {
            return f.clone();
        }
        let font = self.library.as_mut().and_then(|lib| {
            let (loaded, index) = lib.resolve_class(path, "Font")?;
            (loaded.package.export_class(index) == "Font").then_some(())?;
            openxiii_assets::font::Font::read(&loaded.name, &loaded.package, &loaded.data, index, self.config.as_ref().map_or("int", |c| c.language.as_str())).ok()
        });
        let font = font.map(std::rc::Rc::new);
        self.fonts.insert(path.to_string(), font.clone());
        font
    }

    /// Material key of a script value (see `HudDraw`).
    pub(crate) fn material_key(v: &Value) -> String {
        material_path(v)
    }

    /// UMaterial::MaterialUSize / MaterialVSize (vtable +0x70 / +0x74).
    pub(crate) fn material_size(&mut self, material: &Value) -> (i32, i32) {
        let key = Self::material_key(material);
        self.key_size(&key)
    }

    fn key_size(&mut self, key: &str) -> (i32, i32) {
        if key.is_empty() {
            return (0, 0);
        }
        if key == WHITE || key.starts_with("#canvasline:") {
            // FSolidColorTexture: 1 × 1.
            return (1, 1);
        }
        if let Some(rest) = key.strip_prefix("#canvas:") {
            return rest.split_once('|').map_or((0, 0), |(_, inner)| self.key_size(inner));
        }
        if let Some(id) = key.strip_prefix('#').and_then(|n| n.parse::<ObjId>().ok()) {
            return self.object_material_size(id);
        }
        if let Some(&t) = self.canvas_state.sizes_list.get(key) {
            return t;
        }
        let Some(library) = self.library.as_mut() else { return (0, 0) };
        let t = self.canvas_state.materials.size(library, key);
        self.canvas_state.sizes_list.insert(key.to_string(), t);
        t
    }

    /// Size of a material created by the script (UBitmapMaterial: USize; UModifier, UShader,
    /// UCombiner: like `Materials::size`).
    fn object_material_size(&mut self, id: ObjId) -> (i32, i32) {
        if self.objects.get(id).is_none_or(|o| o.is_none()) {
            return (0, 0);
        }
        if self.is_a(id, "BitmapMaterial") {
            return (self.propr(id, "USize").as_int(), self.propr(id, "VSize").as_int());
        }
        let sub = |vm: &mut Vm, name: &str| {
            let v = vm.prop(id, name);
            vm.material_size(&v)
        };
        if self.is_a(id, "Modifier") {
            return sub(self, "Material");
        }
        if self.is_a(id, "Shader") {
            if !matches!(self.prop(id, "Diffuse"), Value::Object(None)) {
                return sub(self, "Diffuse");
            }
            return sub(self, "SelfIllumination");
        }
        if self.is_a(id, "Combiner") {
            let a = sub(self, "Material2");
            let b = sub(self, "Material1");
            return (b.0.max(a.0), b.1.max(a.1));
        }
        (0, 0)
    }

    /// Shared material key of a primitive (created once per name).
    fn name_hud(&mut self, key: &str) -> std::sync::Arc<str> {
        if let Some(n) = self.canvas_state.names_list.get(key) {
            return n.clone();
        }
        let n: std::sync::Arc<str> = key.into();
        self.canvas_state.names_list.insert(n.clone());
        n
    }

    /// Texture coordinates divided by the material's size (FCanvasUtil::DrawTile).
    fn util_uv(&mut self, key: &str, mut uv: [f32; 4]) -> [f32; 4] {
        if !key.is_empty() {
            let (us, vs) = self.key_size(key);
            let fu = 1.0 / us as f32;
            let fv = 1.0 / vs as f32;
            uv = [fu * uv[0], uv[1] * fv, fu * uv[2], fv * uv[3]];
        }
        uv
    }

    /// FCanvasUtil::DrawTile.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn util_tile(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, uv: [f32; 4], key: &str, color: [u8; 4]) -> HudDraw {
        let uv = self.util_uv(key, uv);
        HudDraw::Tile { x1, y1, x2, y2, uv, material: self.name_hud(key), color, point: self.canvas_state.point_filter, enlarged: false, glyph: false }
    }

    /// FCanvasUtil::DrawTriangle.
    fn util_triangle(&mut self, p: [[f32; 2]; 3], uv: [f32; 4], key: &str, color: [u8; 4]) {
        let uv = self.util_uv(key, uv);
        let material = self.name_hud(key);
        self.hud.push(HudDraw::Triangle { p, uv, material, color });
    }

    /// FCanvasUtil::DrawLine: static UFinalBlend on WhiteTextureMaterial,
    /// FrameBufferBlending depending on the style (Masked: alpha test at 127).
    pub(crate) fn util_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: [u8; 4], style: u8) {
        // "#canvasline:{fb}:{test}".
        let key = match style {
            2 => "#canvasline:0:1",
            3 => "#canvasline:4:0",
            4 => "#canvasline:1:0",
            5 => "#canvasline:2:0",
            _ => "#canvasline:0:0",
        };
        let material = self.name_hud(key);
        self.hud.push(HudDraw::Line { from: [x1, y1], to: [x2, y2], material, color });
    }

    /// UCanvas::DrawTile (vtable +0x74).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn canvas_draw_tile(
        &mut self,
        c: ObjId,
        material: &Value,
        x: f32,
        y: f32,
        xl: f32,
        yl: f32,
        u: f32,
        v: f32,
        ul: f32,
        vl: f32,
        _z: f32,
        color: [f32; 4],
        _fog: [f32; 4],
    ) {
        let style = self.canvas_style(c);
        let inner = Self::material_key(material);
        let style = self.menu_tile_style(&inner, style, xl, yl, ul);
        let key = if style != 1 && style != 0 {
            // Static UFinalBlend: ZWrite, ZTest, AlphaTest and TwoSided cleared, then the style.
            let (fb, test) = match style {
                2 => (0, 1),
                3 => (4, 0),
                4 => (1, 0),
                5 => (2, 0),
                _ => (self.canvas_state.blend, 0),
            };
            self.canvas_state.blend = fb;
            format!("#canvas:{fb}:{test}:127|{inner}")
        } else {
            inner
        };
        let v2 = v + vl;
        let u2 = u + ul;
        let y2 = y + yl;
        let x2 = x + xl;
        let col = fcolor(color);
        let uv = [u, v, u2, v2];
        let tile = self.util_tile(x, y, x2, y2, uv, &key, col);
        self.hud.push(tile);
        let dialog = self.pb(c, "bDialogWindow");
        if dialog {
            let d = yl * 0.16;
            self.util_triangle([[x2, y], [(d + x) + xl, yl * 0.66 + y], [x2, y2]], uv, &key, col);
            self.util_triangle([[x, y], [x - d, yl * 0.33 + y], [x, y2]], uv, &key, col);
        }
        if self.pb(c, "bUseBorder") {
            let bc = bgra(&self.prop(c, "BorderColor"));
            let st = style;
            let x0 = x - 0.5;
            let y0 = y - 0.5;
            let r = (x0 + xl) + 0.5;
            let b = (y0 + yl) + 0.5;
            if dialog {
                let rr = r + 1.0;
                self.util_line((x0 + 0.5) - 1.0, y0, rr, y0, bc, st);
                let xl1 = x0 - 1.0;
                self.util_line(xl1, b, (rr as f64 + 1.05) as f32, b, bc, st);
                let h = b - y0;
                let m1 = h * 0.33 + y0;
                let d = h * 0.16;
                let xa = x0 - d;
                self.util_line(x0, y0, xa, m1, bc, st);
                self.util_line(xa, m1, x0, b, bc, st);
                let xa1 = xa - 1.0;
                self.util_line(xl1, y0, xa1, m1, bc, st);
                self.util_line(xa1, m1, xl1, b, bc, st);
                let m2 = h * 0.66 + y0;
                let xb = d + r;
                self.util_line(r, y0, xb, m2, bc, st);
                self.util_line(xb, m2, r, b, bc, st);
                let xb1 = xb + 1.0;
                self.util_line(rr, y0, xb1, m2, bc, st);
                self.util_line(xb1, m2, rr, b, bc, st);
                let y1 = y0 - 1.0;
                let b1 = b + 1.0;
                self.util_line((xl1 as f64 + 0.5) as f32, y1, rr, y1, bc, st);
                self.util_line(xl1, b1, (rr as f64 + 1.0) as f32, b1, bc, st);
            } else {
                self.util_line((x0 as f64 + 0.5) as f32, y0, r, y0, bc, st);
                self.util_line(x0, b, (r as f64 + 1.0) as f32, b, bc, st);
                self.util_line(x0, y0, x0, b, bc, st);
                self.util_line(r, y0, r, b, bc, st);
                let x1 = x0 - 1.0;
                let y1 = y0 - 1.0;
                let r1 = r + 1.0;
                let b1 = b + 1.0;
                self.util_line((x1 as f64 + 0.5) as f32, y1, r1, y1, bc, st);
                self.util_line(x1, b1, (r1 as f64 + 1.0) as f32, b1, bc, st);
                self.util_line(x1, y1, x1, b1, bc, st);
                self.util_line(r1, y1, r1, b1, bc, st);
            }
        }
    }

    /// Text of a font at (x, y) + (OrgX, OrgY), one page cell per
    /// character in the FCanvasUtil of its page (flushed at 100 vertices, then all of them in page
    /// order); `clip`: clipping at ClipX / ClipY; `hotkey`: "&" underlines the next
    /// character. Returns the advance in pixels.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_string(&mut self, c: ObjId, font: &str, x: i32, y: i32, text: &[u8], color: [u8; 4], clip: bool, hotkey: bool) -> i32 {
        let Some(f) = self.font(font) else { return 0 };
        let (org_x, org_y) = (self.cf(c, "OrgX"), self.cf(c, "OrgY"));
        let (clip_x, clip_y) = (self.cf(c, "ClipX"), self.cf(c, "ClipY"));
        let space_x = self.cf(c, "SpaceX");
        let mut utils: Vec<(Vec<HudDraw>, i32)> = vec![(Vec::new(), 0); f.pages.len()];
        let cpp = f.characters_per_page.max(1) as i32;
        let mut pen = 0i32;
        let mut underline = false;
        let mut underline_w = 0i32;
        let mut i = 0usize;
        while i < text.len() && text[i] != 0 {
            let b = text[i];
            let mut hot = false;
            let mut ch = b;
            if hotkey {
                if underline {
                    ch = b'_';
                }
                if b == b'&' {
                    let n = text.get(i + 1).copied().unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    if n != b'&' {
                        hot = true;
                        ch = n;
                    }
                }
            }
            let page = ch as i32 / cpp;
            if let Some(p) = f.pages.get(page as usize).filter(|p| !p.texture.is_empty()) {
                if let Some(g) = p.glyphs.get((ch as i32 - cpp * page) as usize) {
                    let mut w = g.width;
                    if underline && underline_w <= w {
                        w = underline_w;
                    }
                    // The cell's width is trimmed in a copy (esi): the pen's advance
                    // takes the whole width.
                    let mut wc = w;
                    let (mut x0, mut y0, mut u, mut v, mut h) = (x + pen, y, g.u, g.v, g.height);
                    let draw = if !clip {
                        true
                    } else if w + x0 > 0 && (x0 as f32) <= clip_x && y + h > 0 && (y as f32) <= clip_y {
                        if x0 < 0 {
                            u -= x0;
                            wc = w + x0;
                            x0 = 0;
                        }
                        if y0 < 0 {
                            v -= y0;
                            h += y0;
                            y0 = 0;
                        }
                        if ((x0 + wc) as f32) > clip_x {
                            wc = ftol(clip_x - x0 as f32);
                        }
                        if ((y0 + h) as f32) > clip_y {
                            h = ftol(clip_y - y0 as f32);
                        }
                        true
                    } else {
                        false
                    };
                    if draw {
                        let x1 = x0 as f32 + org_x;
                        let y1 = y0 as f32 + org_y;
                        let uv = [u as f32, v as f32, (wc + u) as f32, (h + v) as f32];
                        let mut tile = self.util_tile(x1, y1, wc as f32 + x1, h as f32 + y1, uv, &p.texture, color);
                        // A letter, whatever its filtering (ClippedPrint draws without POINT).
                        if let HudDraw::Tile { glyph, .. } = &mut tile {
                            *glyph = true;
                        }
                        let util = &mut utils[page as usize];
                        if util.1 + 6 > 100 {
                            self.hud.append(&mut util.0);
                            util.1 = 0;
                        }
                        util.0.push(tile);
                        util.1 += 6;
                    }
                    if underline {
                        w = underline_w;
                    }
                    if !hot {
                        pen += ftol(w as f32 + space_x);
                    } else {
                        underline_w = g.width;
                    }
                    underline = hot;
                }
            }
            i += 1;
        }
        for (mut prims, _) in utils {
            self.hud.append(&mut prims);
        }
        pen
    }

    /// Text placed outside the script (the "OpenXIII 0.1" mark of the game launched alone): one page cell
    /// of font `font` per character, at (x, y) in screen pixels. Returns the cells, the
    /// width and the height of the text; nothing if the font is missing.
    pub fn free_text(&mut self, font: &str, x: f32, y: f32, text: &str, color: [u8; 4]) -> (Vec<HudDraw>, f32, f32) {
        let Some(f) = self.font(font) else { return (Vec::new(), 0.0, 0.0) };
        let cpp = f.characters_per_page.max(1) as usize;
        let mut out = Vec::new();
        let (mut pen, mut height) = (0.0f32, 0.0f32);
        // By character (the fonts are indexed by character code, as the script's strings): an
        // accented letter is one glyph, not its UTF-8 bytes.
        for ch in text.chars().map(|c| c as usize) {
            let page = ch / cpp;
            let Some(p) = f.pages.get(page).filter(|p| !p.texture.is_empty()) else { continue };
            let Some(g) = p.glyphs.get(ch - cpp * page) else { continue };
            let (x1, w, h) = (x + pen, g.width as f32, g.height as f32);
            let uv = [g.u as f32, g.v as f32, (g.u + g.width) as f32, (g.v + g.height) as f32];
            let key = p.texture.clone();
            let uv = self.util_uv(&key, uv);
            let material = self.name_hud(&key);
            out.push(HudDraw::Tile { x1, y1: y, x2: x1 + w, y2: y + h, uv, material, color, point: true, enlarged: false, glyph: true });
            pen += w;
            height = height.max(h);
        }
        (out, pen, height)
    }

    /// Filled rectangle placed outside the script (level selection of the game launched alone): WhiteTexture in
    /// FB_AlphaBlend, like a style 5 DrawTile.
    pub fn rectangle_free(x1: f32, y1: f32, x2: f32, y2: f32, color: [u8; 4]) -> HudDraw {
        HudDraw::Tile { x1, y1, x2, y2, uv: [0.0, 0.0, 1.0, 1.0], material: "#canvas:2:0:127|#white".into(), color, point: false, enlarged: false, glyph: false }
    }

    /// A whole game texture ("Package.Group.Name") stretched over (x1, y1)–(x2, y2), placed outside
    /// the script, as a style 1 DrawStretchedTexture (the improved build's Mods page draws the
    /// menus' check mark and arrow this way). None if the texture is not loaded.
    pub fn texture_free(&mut self, path: &str, x1: f32, y1: f32, x2: f32, y2: f32, color: [u8; 4]) -> Option<HudDraw> {
        let (u, v) = self.key_size(path);
        if u <= 0 || v <= 0 {
            return None;
        }
        let uv = self.util_uv(path, [0.0, 0.0, u as f32, v as f32]);
        let material = self.name_hud(path);
        Some(HudDraw::Tile { x1, y1, x2, y2, uv, material, color, point: false, enlarged: false, glyph: false })
    }

    /// The menus' common text drawn at a scale (pages drawn outside the scripts): `free_text`
    /// at (x, y) in screen pixels, `scale` times its size, by the menus' rule (gui.rs,
    /// place_scaled: positions at the real resolution, a whole scale pixel for pixel, any other
    /// bilinear). Returns the cells, the scaled width and height.
    pub fn scaled_text(&mut self, font: &str, x: f32, y: f32, scale: f32, text: &str, color: [u8; 4]) -> (Vec<HudDraw>, f32, f32) {
        let (mut cells, w, h) = self.free_text(font, 0.0, 0.0, text, color);
        for d in &mut cells {
            crate::gui::place_scaled_in(d, [scale; 2], [x, y], crate::gui::enlarged_text_bicubic(self.engine.client.session_graphics));
        }
        (cells, w * scale, h * scale)
    }

    /// Menu cursor placed outside the script (level selection of the game launched alone, drawn over
    /// the pages): position (MouseX, MouseY) of the local GUIController, and its cell as
    /// UGUIController::NativePostRender draws it (MouseCursors[0], style 5, ×SizeX/640) when the game
    /// draws the cursor itself (without the Windows mouse). `w`: width of the screen. In window
    /// pixels: with the improved build's whole-window menu frame (gui.rs, window_menu_frame), the
    /// position and the cell on the menu's page (`cursor_menu_page`) placed by the frame.
    pub fn cursor_menu_draw(&mut self, w: u32) -> Option<((f32, f32), Option<HudDraw>)> {
        let Some(f) = self.menu_layout.last_frame else { return self.cursor_menu_page(w) };
        let (pos, tile) = self.cursor_menu_page(f.virtual_size.0.max(1) as u32)?;
        let tile = tile.map(|mut t| {
            f.place(&mut t);
            t
        });
        Some((f.to_canvas(pos.0, pos.1), tile))
    }

    /// `cursor_menu_draw` on the menu's page itself (the GUI's MouseX / MouseY, a page `w` pixels
    /// wide), for a page drawn on it and placed by the frame afterwards (Vm::menu_screen).
    pub fn cursor_menu_page(&mut self, w: u32) -> Option<((f32, f32), Option<HudDraw>)> {
        let ctrl = self.gui_local()?;
        let pos = (self.propr(ctrl, "MouseX").as_int() as f32, self.propr(ctrl, "MouseY").as_int() as f32);
        let mouse_windows = self.propr(ctrl, "ViewportOwner").as_object().is_some_and(|p| self.propr(p, "bWindowsMouseAvailable").as_bool());
        let cursor = match self.prop(ctrl, "MouseCursors") {
            Value::Array(m) => m.first().cloned().unwrap_or(Value::Object(None)),
            _ => Value::Object(None),
        };
        if mouse_windows || !cursor.as_bool() || self.touch_hides_cursor {
            return Some((pos, None));
        }
        let offset = match self.prop(ctrl, "MouseCursorOffset") {
            Value::Array(m) => m.first().map(|v| v.as_vector()).unwrap_or([0.0; 3]),
            _ => [0.0; 3],
        };
        let (u, v) = self.material_size(&cursor);
        let scale = if w < 640 { 1.0 } else { (w / 640) as f32 };
        let (cw, ch) = (u as f32 * scale, v as f32 * scale);
        let (x, y) = (ftol(pos.0 - offset[0] * cw) as f32, ftol(pos.1 - offset[1] * ch) as f32);
        let key = format!("#canvas:2:0:127|{}", Self::material_key(&cursor));
        let uv = self.util_uv(&key, [0.0, 0.0, u as f32, v as f32]);
        let material = self.name_hud(&key);
        Some((pos, Some(HudDraw::Tile { x1: x, y1: y, x2: x + cw, y2: y + ch, uv, material, color: [255; 4], point: false, enlarged: false, glyph: false })))
    }

    /// UCanvas::WrappedPrint: cuts the text into lines that fit in ClipX (at
    /// spaces, or at the last character that fits for a first word that is too long), draws each
    /// line if `draw` (below the bottom of the viewport and above the screen excluded), centred if
    /// `center`; CurX = 0 and CurY goes down by each line. Returns (XL, YL).
    /// WrappedPrintf (Canvas vtable +0x90) of the loading animation (loading_anim.rs).
    pub(crate) fn loading_print(&mut self, c: ObjId, font: &str, text: &[u8]) {
        let _ = self.wrapped_print(c, true, Some(font), false, text);
    }

    fn wrapped_print(&mut self, c: ObjId, draw: bool, font: Option<&str>, center: bool, text: &[u8]) -> (i32, i32) {
        if self.cf(c, "ClipX") < 0.0 || self.cf(c, "ClipY") < 0.0 {
            return (0, 0);
        }
        let Some(font) = font else { return (0, 0) };
        let Some(f) = self.font(font) else { return (0, 0) };
        // FRenderInterface +0xAC(0): text with point filtering, +0xAC(1) on exit.
        self.canvas_state.point_filter = true;
        let r = self.wrapped_print_lines(c, draw, font, f, center, text);
        self.canvas_state.point_filter = false;
        r
    }

    fn wrapped_print_lines(&mut self, c: ObjId, draw: bool, font: &str, f: std::rc::Rc<openxiii_assets::font::Font>, center: bool, text: &[u8]) -> (i32, i32) {
        let (mut xl, mut yl) = (0i32, 0i32);
        let lines_list = wrapped_lines(&f, self.cf(c, "SpaceX"), self.cf(c, "SpaceY"), self.cf(c, "ClipX"), self.cf(c, "CurX"), text);
        for (t, last, line_w, line_h) in lines_list {
            if draw {
                let base = self.cf(c, "OrgY") + self.cf(c, "CurY");
                if base < self.viewport.1 as f32 && line_h as f32 + base > 0.0 {
                    let line = &text[t..t + last];
                    let cur_x = self.cf(c, "CurX");
                    let x = if center { ftol((self.cf(c, "ClipX") - line_w as f32) * 0.5 + cur_x) } else { ftol(cur_x) };
                    let y = ftol(self.cf(c, "CurY"));
                    let color = self.draw_color(c);
                    let adv = self.draw_string(c, font, x, y, line, color, false, false);
                    self.set_cf(c, "CurX", (x + adv) as f32);
                }
            }
            self.set_cf(c, "CurX", 0.0);
            let cy = self.cf(c, "CurY");
            self.set_cf(c, "CurY", line_h as f32 + cy);
            yl += line_h;
            xl = xl.max(line_w);
        }
        (xl, yl)
    }

    /// UCanvas::WrappedStrLenf (vtable +0x8C): WrappedPrint without drawing.
    #[allow(dead_code)] // Called by GUI.dll (UCanvas vtable).
    pub(crate) fn canvas_wrapped_str_len(&mut self, c: ObjId, font: &Value, text: &str) -> (i32, i32) {
        let font = Self::font_path(font);
        self.wrapped_print(c, false, font.as_deref(), false, &ansi(text))
    }

    /// UCanvas::WrappedPrintf (vtable +0x90): WrappedPrint in style 1.
    #[allow(dead_code)] // Called by GUI.dll (UCanvas vtable).
    pub(crate) fn canvas_wrapped_printf(&mut self, c: ObjId, font: &Value, center: bool, text: &str) {
        let font = Self::font_path(font);
        self.wrapped_print(c, true, font.as_deref(), center, &ansi(text));
    }

    /// UCanvas::ClippedStrLen (vtable +0x98): sum of the widths (plus SpaceX
    /// except after the last character) × scale, largest height × scale.
    pub(crate) fn canvas_clipped_str_len(&mut self, c: ObjId, font: &Value, scale_x: f32, scale_y: f32, text: &str) -> (i32, i32) {
        let Some(font) = Self::font_path(font).or_else(|| self.canvas_font(c)) else { return (0, 0) };
        let Some(f) = self.font(&font) else { return (0, 0) };
        clipped_str_len(&f, self.cf(c, "SpaceX"), scale_x, scale_y, &ansi(text))
    }

    /// UCanvas::ClippedPrint (vtable +0x9C): text clipped at (CurX, CurY), colour
    /// DrawColor passed through an FPlane; the scale and centring are not read.
    pub(crate) fn canvas_clipped_print(&mut self, c: ObjId, font: &Value, _scale_x: f32, _scale_y: f32, _center: bool, text: &str) {
        let Some(font) = Self::font_path(font) else { return };
        let color = fcolor(fplane(self.draw_color(c)));
        let y = ftol(self.cf(c, "CurY"));
        let x = ftol(self.cf(c, "CurX"));
        self.draw_string(c, &font, x, y, &ansi(text), color, true, false);
    }

    /// UCanvas::DrawTextJustified (vtable +0xB4): ClippedPrint in the rectangle
    /// (x1, y1)-(x2, y2), centred vertically, left / centred / right.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn canvas_draw_text_justified(&mut self, c: ObjId, justification: u8, x1: f32, y1: f32, x2: f32, y2: f32, text: &str) {
        self.set_cf(c, "CurX", 0.0);
        self.set_cf(c, "CurY", 0.0);
        let font = self.prop(c, "Font");
        let (xl, yl) = self.canvas_clipped_str_len(c, &font, 1.0, 1.0, text);
        let h = y2 - y1;
        let (cx, cy) = place_justified(xl, yl, justification, x1, y1, x2, y2);
        self.set_cf(c, "CurY", cy);
        self.set_cf(c, "CurX", cx);
        let saved = [ftol(self.cf(c, "ClipX")), ftol(self.cf(c, "ClipY")), ftol(self.cf(c, "OrgX")), ftol(self.cf(c, "OrgY"))];
        self.set_cf(c, "ClipX", x2 - x1);
        self.set_cf(c, "OrgY", y1);
        self.set_cf(c, "ClipY", h);
        self.set_cf(c, "OrgX", x1);
        self.canvas_clipped_print(c, &font, 1.0, 1.0, false, text);
        for (k, n) in ["ClipX", "ClipY", "OrgX", "OrgY"].iter().enumerate() {
            self.set_cf(c, n, saved[k] as f32);
        }
    }

    /// UCanvas::WrapStringToArray (vtable +0x94): lines of at most `width`
    /// pixels, cut at spaces (or at the last character that fits), empty lines for each
    /// leading `eol`.
    pub(crate) fn canvas_wrap_string_to_array(&mut self, c: ObjId, text: &str, width: f32, font: &Value, eol: char) -> Vec<String> {
        let mut out = Vec::new();
        let Some(font) = Self::font_path(font).or_else(|| self.canvas_font(c)) else { return out };
        let Some(f) = self.font(&font) else { return out };
        let text = ansi(text);
        let eol = if (eol as u32) < 256 { eol as u8 } else { b'?' };
        let at = |k: usize| text.get(k).copied().unwrap_or(0);
        let mut s = 0usize;
        while at(s) != 0 {
            while at(s) == eol && eol != 0 {
                out.push(String::new());
                s += 1;
            }
            if at(s) == 0 {
                break;
            }
            let mut last = 0usize;
            let mut w_sum = 0i32;
            let mut got = false;
            let mut i = 0usize;
            loop {
                let ch = at(s + i);
                if ch == eol {
                    break;
                }
                let (w, _) = char_size(&f, ch);
                w_sum += ftol(w as f32 + self.cf(c, "SpaceX"));
                if w_sum as f32 > width {
                    break;
                }
                let next = at(s + i + 1);
                i += 1;
                let brk = next == b' ' || next == eol || next == 0;
                if brk {
                    last = i;
                    got = true;
                } else if !got {
                    last = i;
                }
                if next == 0 {
                    break;
                }
            }
            if last == 0 {
                if at(s) != 0 {
                    out.push(from_ansi(&[at(s)]));
                    s += 1;
                }
            } else {
                out.push(from_ansi(&text[s..s + last]));
            }
            s += last;
            while at(s) == b' ' {
                s += 1;
            }
            if at(s) == eol {
                s += 1;
            }
        }
        out
    }

    /// UCanvas::DrawTileStretched (vtable +0xA0): nine pieces, corners at
    /// half of the texture (minus half a texel), edges and centre stretched over one column or
    /// one row of a texel.
    pub(crate) fn canvas_draw_tile_stretched(&mut self, c: ObjId, material: &Value, x: f32, y: f32, xl: f32, yl: f32) {
        self.set_cf(c, "CurX", x);
        self.set_cf(c, "CurY", y);
        if Self::material_key(material).is_empty() {
            // GLog, like DrawTileScaled.
            self.log.push("Log: DrawTile: Missing Material".to_string());
            return;
        }
        if self.canvas_style(c) == 0 {
            return;
        }
        let (us, vs) = self.material_size(material);
        let mw = us as f32;
        let mh = vs as f32;
        let hw = fistp(mw * 0.5 - 0.5) as f32;
        let hh = fistp(mh * 0.5 - 0.5) as f32;
        let dw = xl - mw;
        let dh = yl - mh;
        let tile = |vm: &mut Vm, tx: f32, ty: f32, txl: f32, tyl: f32, u: f32, v: f32, ul: f32, vl: f32| {
            let col = fplane(vm.draw_color(c));
            vm.canvas_draw_tile(c, material, tx, ty, txl, tyl, u, v, ul, vl, 0.0, col, [0.0; 4]);
        };
        let cx = |vm: &Vm| vm.cf(c, "CurX");
        let cy = |vm: &Vm| vm.cf(c, "CurY");
        let corner_w;
        let edge_h;
        if mw < xl {
            corner_w = hw;
            let edge_h = if mh <= yl { hh } else { yl * 0.5 };
            let (x0, y0) = (cx(self), cy(self));
            tile(self, hw + x0, y0, dw, edge_h, hw, 0.0, 1.0, edge_h);
            let (x0, y0) = (cx(self), cy(self));
            tile(self, hw + x0, (yl + y0) - edge_h, dw, edge_h, hw, mh - edge_h, 1.0, edge_h);
        } else {
            corner_w = xl * 0.5;
        }
        if mh < yl {
            edge_h = hh;
            let (x0, y0) = (cx(self), cy(self));
            tile(self, x0, hh + y0, corner_w, dh, 0.0, hh, corner_w, 1.0);
            let (x0, y0) = (cx(self), cy(self));
            tile(self, (xl + x0) - corner_w, hh + y0, corner_w, dh, mw - corner_w, hh, corner_w, 1.0);
        } else {
            edge_h = yl * 0.5;
        }
        if mh < yl && mw < xl {
            let (x0, y0) = (cx(self), cy(self));
            tile(self, corner_w + x0, edge_h + y0, dw, dh, corner_w, edge_h, 1.0, 1.0);
        }
        let (x0, y0) = (cx(self), cy(self));
        tile(self, x0, y0, corner_w, edge_h, 0.0, 0.0, corner_w, edge_h);
        let u_r = mw - corner_w;
        let (x0, y0) = (cx(self), cy(self));
        tile(self, (xl + x0) - corner_w, y0, corner_w, edge_h, u_r, 0.0, corner_w, edge_h);
        let v_b = mh - edge_h;
        let (x0, y0) = (cx(self), cy(self));
        tile(self, x0, (yl + y0) - edge_h, corner_w, edge_h, 0.0, v_b, corner_w, edge_h);
        let (x0, y0) = (cx(self), cy(self));
        tile(self, (xl + x0) - corner_w, (yl + y0) - edge_h, corner_w, edge_h, u_r, v_b, corner_w, edge_h);
    }

    /// UCanvas::DrawTileScaled (vtable +0xA4): the whole texture, size × scale.
    pub(crate) fn canvas_draw_tile_scaled(&mut self, c: ObjId, material: &Value, x: f32, y: f32, xl: f32, yl: f32) {
        if Self::material_key(material).is_empty() {
            self.log.push("Log: DrawTile: Missing Material".to_string());
            return;
        }
        if self.canvas_style(c) == 0 {
            return;
        }
        let (us, vs) = self.material_size(material);
        let col = fplane(self.draw_color(c));
        self.canvas_draw_tile(c, material, x, y, us as f32 * xl, vs as f32 * yl, 0.0, 0.0, us as f32, vs as f32, 0.0, col, [0.0; 4]);
    }

    /// UCanvas::DrawTileBound (vtable +0xA8): texels = pixels.
    pub(crate) fn canvas_draw_tile_bound(&mut self, c: ObjId, material: &Value, x: f32, y: f32, xl: f32, yl: f32) {
        if Self::material_key(material).is_empty() {
            self.log.push("Log: DrawTileBound: Missing Material".to_string());
        }
        if self.canvas_style(c) != 0 {
            let col = fplane(self.draw_color(c));
            self.canvas_draw_tile(c, material, x, y, xl, yl, 0.0, 0.0, xl, yl, 0.0, col, [0.0; 4]);
        }
    }

    /// UCanvas::DrawTileScaleBound (vtable +0xB0): the whole texture within (xl, yl).
    #[allow(dead_code)] // Called by GUI.dll (UCanvas vtable).
    pub(crate) fn canvas_draw_tile_scale_bound(&mut self, c: ObjId, material: &Value, x: f32, y: f32, xl: f32, yl: f32) {
        if Self::material_key(material).is_empty() {
            // The engine then calls MaterialUSize on the null pointer.
            self.log.push("Log: DrawTileScaleBound: Missing Material".to_string());
            return;
        }
        if self.canvas_style(c) != 0 {
            let (us, vs) = self.material_size(material);
            let col = fplane(self.draw_color(c));
            self.canvas_draw_tile(c, material, x, y, xl, yl, 0.0, 0.0, us as f32, vs as f32, 0.0, col, [0.0; 4]);
        }
    }

    /// UCanvas::DrawTileJustified (vtable +0xAC): the whole texture within (xl, yl)
    /// without distorting it, aligned left / centre / right (or top… bottom).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn canvas_draw_tile_justified(&mut self, c: ObjId, material: &Value, x: f32, y: f32, xl: f32, yl: f32, justification: u8) {
        if Self::material_key(material).is_empty() {
            // Same text as DrawTileScaleBound.
            self.log.push("Log: DrawTileScaleBound: Missing Material".to_string());
            return;
        }
        if self.canvas_style(c) == 0 {
            return;
        }
        let (us, vs) = self.material_size(material);
        let mw = us as f32;
        let mh = vs as f32;
        if mw <= 0.0 || mh <= 0.0 {
            return;
        }
        let r = mh / mw;
        let rr = yl / xl;
        let col = fplane(self.draw_color(c));
        if r == rr {
            self.canvas_draw_tile(c, material, x, y, xl, yl, 0.0, 0.0, mw, mh, 0.0, col, [0.0; 4]);
        } else if rr < r {
            let w = (rr / r) * xl;
            let tx = match justification {
                1 => ((xl - w) as f64 * 0.5) as f32 + x,
                2 => (xl - w) + x,
                _ => x,
            };
            self.canvas_draw_tile(c, material, tx, y, w, yl, 0.0, 0.0, mw, mh, 0.0, col, [0.0; 4]);
        } else {
            let h = (r / rr) * yl;
            let ty = match justification {
                1 => ((yl - h) as f64 * 0.5) as f32 + y,
                2 => (yl - h) + y,
                _ => y,
            };
            self.canvas_draw_tile(c, material, x, ty, xl, h, 0.0, 0.0, mw, mh, 0.0, col, [0.0; 4]);
        }
    }

    /// UCanvas::DrawFrame: black frame of the cutscenes, white edging, as
    /// triangles of WhiteSquareTexture.
    pub(crate) fn canvas_draw_frame(&mut self, c: ObjId, p1: f32, p2: f32, p3: f32, p4: f32) {
        const TEX: &str = "Engine.WhiteSquareTexture";
        const BLACK: [u8; 4] = [0, 0, 0, 255];
        const WHITE_C: [u8; 4] = [255; 4];
        let clip_x = self.cf(c, "ClipX");
        let clip_y = self.cf(c, "ClipY");
        let e1 = clip_x * 0.005;
        let e2 = clip_x * 0.01;
        let max0 = |v: f32| if v < 0.0 { 0.0 } else { v };
        let min_to = |v: f32, m: f32| if v <= m { v } else { m };
        let hgt = clip_y - 1.0;
        let t0 = max0(p1 * hgt);
        let t1 = max0(t0 - e1);
        let t2 = max0(t1 - e2);
        let b0 = min_to(hgt * p2, hgt);
        let b1 = min_to(b0 + e1, hgt);
        let b2 = min_to(b1 + e2, hgt);
        let wdt = clip_x - 1.0;
        let l0 = max0(p3 * wdt);
        let l1 = max0(l0 - e1);
        let l2 = max0(l1 - e2);
        let r0 = min_to(wdt * p4, wdt);
        let r1 = min_to(r0 + e1, wdt);
        let r2 = min_to(r1 + e2, wdt);
        let tri = |vm: &mut Vm, a: [f32; 2], b: [f32; 2], d: [f32; 2], col: [u8; 4]| vm.util_triangle([a, b, d], [0.0; 4], TEX, col);
        if 0.0 <= t0 {
            tri(self, [l1, t1], [l1, t0], [r1, t0], BLACK);
            tri(self, [l1, t1], [r1, t1], [r1, t0], BLACK);
            if 0.0 <= t1 {
                tri(self, [l2, t2], [l2, t1], [r2, t1], WHITE_C);
                tri(self, [l2, t2], [r2, t2], [r2, t1], WHITE_C);
                if 0.0 <= t2 {
                    tri(self, [0.0, 0.0], [0.0, t2], [clip_x, t2], BLACK);
                    tri(self, [0.0, 0.0], [clip_x, 0.0], [clip_x, t2], BLACK);
                }
            }
        }
        if b0 < clip_y {
            tri(self, [l1, b1], [l1, b0], [r1, b0], BLACK);
            tri(self, [l1, b1], [r1, b1], [r1, b0], BLACK);
            if b1 < clip_y {
                tri(self, [l2, b2], [l2, b1], [r2, b1], WHITE_C);
                tri(self, [l2, b2], [r2, b2], [r2, b1], WHITE_C);
                if b2 < clip_y {
                    tri(self, [0.0, clip_y], [0.0, b2], [clip_x, b2], BLACK);
                    tri(self, [0.0, clip_y], [clip_x, clip_y], [clip_x, b2], BLACK);
                }
            }
        }
        if 0.0 <= l0 {
            tri(self, [l1, t1], [l0, t1], [l0, b1], BLACK);
            tri(self, [l1, b1], [l1, t1], [l0, b1], BLACK);
            if 0.0 <= l1 {
                tri(self, [l2, t2], [l1, t2], [l1, b2], WHITE_C);
                tri(self, [l2, b2], [l2, t2], [l1, b2], WHITE_C);
                if 0.0 <= l2 {
                    tri(self, [0.0, 0.0], [l2, 0.0], [l2, clip_y], BLACK);
                    tri(self, [0.0, clip_y], [0.0, 0.0], [l2, clip_y], BLACK);
                }
            }
        }
        if r0 < clip_x {
            tri(self, [r1, t1], [r0, t1], [r0, b1], BLACK);
            tri(self, [r1, b1], [r1, t1], [r0, b1], BLACK);
            if r1 < clip_x {
                tri(self, [r2, t2], [r1, t2], [r1, b2], WHITE_C);
                tri(self, [r2, b2], [r2, t2], [r1, b2], WHITE_C);
                if r2 < clip_x {
                    tri(self, [clip_x, 0.0], [r2, 0.0], [r2, clip_y], BLACK);
                    tri(self, [clip_x, clip_y], [clip_x, 0.0], [r2, clip_y], BLACK);
                }
            }
        }
    }

    /// UCanvas::execDrawMsgboxBackground: white box outlined in black (and
    /// bands around it if `!only_center`), as DrawTileBound of WhiteTextureMaterial.
    #[allow(clippy::too_many_arguments)]
    fn canvas_msgbox_background(&mut self, c: ObjId, only_center: bool, ox: f32, oy: f32, lw: f32, lh: f32, w: f32, h: f32) {
        let white = Value::Asset(WHITE.to_string());
        let black_c = color_value([0, 0, 0, 255]);
        let white_c = color_value([255; 4]);
        let bound = |vm: &mut Vm, x: f32, y: f32, xl: f32, yl: f32| vm.canvas_draw_tile_bound(c, &white, x, y, xl, yl);
        let bands_start = self.begin_message_bands();
        if !only_center {
            let lh4 = lh + 4.0;
            self.set_prop(c, "DrawColor", black_c.clone());
            let clip_x = self.cf(c, "ClipX");
            let low = |vm: &Vm| crate::ai_move::fmul_d(vm.cf(c, "ClipY"), 0.95) - oy;
            bound(self, 0.0, (oy - lh) - 2.0, clip_x, lh4);
            bound(self, (w + ox) - 2.0, oy, lw + 4.0, h);
            bound(self, ox, (h + oy) - 2.0, clip_x - ox, lh + 4.0);
            let l = low(self);
            bound(self, (ox - lw) - 2.0, oy, lw + 4.0, l);
            self.set_prop(c, "DrawColor", white_c.clone());
            bound(self, 0.0, oy - lh, clip_x, lh);
            bound(self, w + ox, oy, lw, h);
            bound(self, ox, h + oy, clip_x - ox, lh);
            let l = low(self);
            bound(self, ox - lw, oy, lw, l);
            self.finish_message_bands(bands_start);
        }
        self.set_prop(c, "DrawColor", white_c);
        bound(self, ox, oy, w, h);
        self.set_prop(c, "DrawColor", black_c);
        bound(self, ox - 1.0, oy - 1.0, w + 2.0, 2.0);
        bound(self, ox - 1.0, oy - 1.0, 2.0, h + 2.0);
        bound(self, ox - 1.0, (h + oy) - 1.0, w + 2.0, 2.0);
        bound(self, (w + ox) - 1.0, oy - 1.0, 2.0, h + 2.0);
    }

    /// UCanvas::Update: Reset event (DealWithResetEvent), then ClipX / ClipY
    /// and SizeX / SizeY of the viewport.
    pub fn canvas_update(&mut self, c: ObjId) {
        self.event_pub(c, "Reset", vec![]);
        let clip_x = self.viewport.0 as f32;
        self.set_cf(c, "ClipX", clip_x);
        self.set_prop(c, "SizeX", Value::Int(ftol(clip_x)));
        let clip_y = self.viewport.1 as f32;
        self.set_cf(c, "ClipY", clip_y);
        self.set_prop(c, "SizeY", Value::Int(ftol(clip_y)));
    }

    /// UCanvas::execDealWithResetEvent: the class's default values for
    /// Font, SpaceX/Y, OrgX/Y, CurX/Y, Style, DrawColor and CurYL; bCenter and bNoSmooth false,
    /// Z = 1.
    fn canvas_reset(&mut self, c: ObjId) {
        let Some(class) = self.objects[c].as_ref().map(|o| o.class) else { return };
        let cdo = self.classes[class].cdo;
        for n in ["Font", "SpaceX", "SpaceY", "OrgX", "OrgY", "CurX", "CurY", "Style", "DrawColor", "CurYL"] {
            let v = self.prop(cdo, n);
            self.set_prop(c, n, v);
        }
        self.set_prop(c, "bCenter", Value::Bool(false));
        self.set_prop(c, "bNoSmooth", Value::Bool(false));
        self.set_cf(c, "Z", 1.0);
    }

    /// Canvas natives (UCanvas::exec*); `None` if `name` is not one of them.
    pub(crate) fn canvas_native(&mut self, name: &str, c: ObjId, args: &mut [Value], supplied: &[bool]) -> Option<Value> {
        let f = |i: usize| args.get(i).map(|v| v.as_float()).unwrap_or(0.0);
        let given = |i: usize| supplied.get(i).copied().unwrap_or(false);
        Some(match name {
            // UCanvas::execSetPos.
            "SetPos" => {
                self.set_cf(c, "CurX", f(0));
                self.set_cf(c, "CurY", f(1));
                Value::Void
            }
            // UCanvas::execSetOrigin.
            "SetOrigin" => {
                self.set_cf(c, "OrgX", f(0));
                self.set_cf(c, "OrgY", f(1));
                Value::Void
            }
            // UCanvas::execSetClip.
            "SetClip" => {
                self.set_cf(c, "ClipX", f(0));
                self.set_cf(c, "ClipY", f(1));
                Value::Void
            }
            // UCanvas::execSetDrawColor (execMakeColor).
            "SetDrawColor" | "MakeColor" => {
                // A is 255 if the argument is missing.
                let a = if given(3) { args[3].as_byte() } else { 255 };
                let color = color_value([args[2].as_byte(), args[1].as_byte(), args[0].as_byte(), a]);
                if name == "MakeColor" {
                    return Some(color);
                }
                self.set_prop(c, "DrawColor", color);
                Value::Void
            }
            // UCanvas::execGetScreenHeight.
            "GetScreenHeight" => Value::Int(480),
            // UCanvas::execDealWithResetEvent.
            "DealWithResetEvent" => {
                self.canvas_reset(c);
                Value::Void
            }
            // UCanvas::execStrLen.
            "StrLen" => {
                let text = ansi(&args[0].as_name());
                let (sx, sy) = (ftol(self.cf(c, "CurX")), ftol(self.cf(c, "CurY")));
                self.set_cf(c, "CurX", 0.0);
                self.set_cf(c, "CurY", 0.0);
                let font = self.canvas_font(c);
                let (xl, yl) = self.wrapped_print(c, false, font.as_deref(), false, &text);
                self.set_cf(c, "CurY", sy as f32);
                self.set_cf(c, "CurX", sx as f32);
                if args.len() > 2 {
                    args[1] = Value::Float(xl as f32);
                    args[2] = Value::Float(yl as f32);
                }
                Value::Void
            }
            // UCanvas::execTextSize.
            "TextSize" => {
                let Some(font) = self.canvas_font(c).and_then(|p| self.font(&p)) else {
                    self.warn("TextSize: No font".to_string());
                    return Some(Value::Void);
                };
                let (mut xl, mut yl) = (0i32, 0i32);
                for ch in ansi(&args[0].as_name()) {
                    let (w, h) = char_size(&font, ch);
                    xl += w;
                    if yl < h {
                        yl = h;
                    }
                }
                if args.len() > 2 {
                    args[1] = Value::Float(xl as f32);
                    args[2] = Value::Float(yl as f32);
                }
                Value::Void
            }
            // UCanvas::execDrawText.
            "DrawText" => {
                let text = ansi(&args[0].as_name());
                let cr = if given(1) { args[1].as_bool() } else { true };
                let Some(font) = self.canvas_font(c) else {
                    self.warn("DrawText: No font".to_string());
                    return Some(Value::Void);
                };
                let help_bar = self.begin_canvas_text(c);
                let (mut xl, mut yl) = (0, 0);
                let style = self.canvas_style(c);
                if style != 0 {
                    let center = self.pb(c, "bCenter");
                    if self.pb(c, "bTextShadow") {
                        let (ox, oy) = (self.cf(c, "CurX"), self.cf(c, "CurY"));
                        let color = self.prop(c, "DrawColor");
                        self.set_cf(c, "CurX", ox + 1.0);
                        self.set_cf(c, "CurY", oy + 1.0);
                        let k = bgra(&color);
                        let dim = |v: u8| ftol(v as f32 * 0.3) as u8;
                        self.set_prop(c, "DrawColor", color_value([dim(k[0]), dim(k[1]), dim(k[2]), dim(k[3])]));
                        self.wrapped_print(c, true, Some(&font), center, &text);
                        self.set_prop(c, "DrawColor", color);
                        self.set_cf(c, "CurX", ox);
                        self.set_cf(c, "CurY", oy);
                    }
                    (xl, yl) = self.wrapped_print(c, true, Some(&font), center, &text);
                }
                let cx = self.cf(c, "CurX");
                self.set_cf(c, "CurX", xl as f32 + cx);
                let cyl = self.cf(c, "CurYL");
                self.set_cf(c, "CurYL", if cyl < yl as f32 { yl as f32 } else { cyl });
                if cr {
                    self.set_cf(c, "CurX", 0.0);
                    let (cyl, cy) = (self.cf(c, "CurYL"), self.cf(c, "CurY"));
                    self.set_cf(c, "CurY", cyl + cy);
                    self.set_cf(c, "CurYL", 0.0);
                }
                self.finish_canvas_text(c, help_bar);
                Value::Void
            }
            // UCanvas::execDrawTextClipped.
            "DrawTextClipped" => {
                let text = ansi(&args[0].as_name());
                let hotkey = given(1) && args[1].as_bool();
                let Some(font) = self.canvas_font(c) else {
                    self.warn("DrawTextClipped: No font".to_string());
                    return Some(Value::Void);
                };
                let color = self.draw_color(c);
                let y = ftol(self.cf(c, "CurY"));
                let x = ftol(self.cf(c, "CurX"));
                // FRenderInterface +0xAC(0) then (1) around the text.
                self.canvas_state.point_filter = true;
                self.draw_string(c, &font, x, y, &text, color, true, hotkey);
                self.canvas_state.point_filter = false;
                Value::Void
            }
            // UCanvas::execDrawTextJustified.
            "DrawTextJustified" => {
                let text = args[0].as_name();
                self.canvas_draw_text_justified(c, args[1].as_byte(), f(2), f(3), f(4), f(5), &text);
                Value::Void
            }
            // UCanvas::execDrawTile.
            "DrawTile" => {
                let k = self.draw_color(c);
                // Mods build: a weapon's TracerColor (mod_weapons.rs).
                #[cfg(feature = "mods")]
                let k = self.mod_tracer_draw_color(k);
                if self.canvas_style(c) == 3 && k[0] == 0 && k[1] == 0 && k[2] == 0 {
                    // Translucent and black: nothing (the cursor does not move).
                    return Some(Value::Void);
                }
                if Self::material_key(&args[0]).is_empty() {
                    self.warn("DrawTile: Missing Texture".to_string());
                    return Some(Value::Void);
                }
                let (xl, yl) = (f(1), f(2));
                // Mods build: the tracer of a weapon held with a FirstPersonModel ends at that
                // model's muzzle (mod_weapons.rs).
                #[cfg(feature = "mods")]
                let (xl, yl) = self.mod_tracer_size(c, &args[0], xl, yl);
                if self.canvas_style(c) != 0 {
                    let (x, y) = (self.cf(c, "CurX") + self.cf(c, "OrgX"), self.cf(c, "CurY") + self.cf(c, "OrgY"));
                    let z = self.cf(c, "Z");
                    let mat = args[0].clone();
                    // Mods build: the HUD icon of a mod's new weapon instead of its slot's tile
                    // (pickup message, HudMessage.PostRender), mod_weapons.rs.
                    #[cfg(feature = "mods")]
                    if let Some((icon, [u, v, ul, vl])) = self.mod_hud_tile(&mat, f(3), f(4), f(5), f(6)) {
                        self.canvas_draw_tile(c, &icon, x, y, xl, yl, u, v, ul, vl, z, fplane(k), [0.0; 4]);
                        let (sx, cx) = (self.cf(c, "SpaceX"), self.cf(c, "CurX"));
                        self.set_cf(c, "CurX", sx + cx + xl);
                        let cyl = self.cf(c, "CurYL");
                        self.set_cf(c, "CurYL", if cyl < yl { yl } else { cyl });
                        return Some(Value::Void);
                    }
                    self.canvas_draw_tile(c, &mat, x, y, xl, yl, f(3), f(4), f(5), f(6), z, fplane(k), [0.0; 4]);
                }
                let (sx, cx) = (self.cf(c, "SpaceX"), self.cf(c, "CurX"));
                self.set_cf(c, "CurX", sx + cx + xl);
                let cyl = self.cf(c, "CurYL");
                self.set_cf(c, "CurYL", if cyl < yl { yl } else { cyl });
                Value::Void
            }
            // UCanvas::execDrawTileClipped.
            "DrawTileClipped" => {
                if Self::material_key(&args[0]).is_empty() {
                    self.warn("DrawTileClipped: Missing Texture".to_string());
                    return Some(Value::Void);
                }
                let (mut xl, mut yl, mut u, mut v, mut ul, mut vl) = (f(1), f(2), f(3), f(4), f(5), f(6));
                if !(0.0 < xl && 0.0 < yl) {
                    return Some(Value::Void);
                }
                let mut cx = self.cf(c, "CurX");
                let mut cy = self.cf(c, "CurY");
                if cx < 0.0 {
                    let t = (cx / xl) * ul;
                    u -= t;
                    ul = t + ul;
                    xl += cx;
                    cx = 0.0;
                    self.set_cf(c, "CurX", 0.0);
                }
                if cy < 0.0 {
                    let t = (cy / yl) * vl;
                    v -= t;
                    vl = t + vl;
                    yl += cy;
                    cy = 0.0;
                    self.set_cf(c, "CurY", 0.0);
                }
                let (clip_x, clip_y) = (self.cf(c, "ClipX"), self.cf(c, "ClipY"));
                let w = clip_x - cx;
                if w < xl {
                    ul = ((w - xl) / xl + 1.0) * ul;
                    xl = clip_x - cx;
                }
                let h = clip_y - cy;
                if h < yl {
                    vl = ((h - yl) / yl + 1.0) * vl;
                    yl = clip_y - cy;
                }
                if self.canvas_style(c) != 0 {
                    let k = self.draw_color(c);
                    let (x, y) = (cx + self.cf(c, "OrgX"), cy + self.cf(c, "OrgY"));
                    let z = self.cf(c, "Z");
                    let mat = args[0].clone();
                    self.canvas_draw_tile(c, &mat, x, y, xl, yl, u, v, ul, vl, z, fplane(k), [0.0; 4]);
                }
                let sx = self.cf(c, "SpaceX");
                self.set_cf(c, "CurX", sx + cx + xl);
                let cyl = self.cf(c, "CurYL");
                self.set_cf(c, "CurYL", if cyl < yl { yl } else { cyl });
                Value::Void
            }
            // UCanvas::execDrawTileStretched (execDrawTileScaled).
            "DrawTileStretched" | "DrawTileScaled" => {
                let mat = args[0].clone();
                let (x, y) = (self.cf(c, "CurX"), self.cf(c, "CurY"));
                if name == "DrawTileStretched" {
                    self.canvas_draw_tile_stretched(c, &mat, x, y, f(1), f(2));
                } else {
                    self.canvas_draw_tile_scaled(c, &mat, x, y, f(1), f(2));
                }
                Value::Void
            }
            // UCanvas::execDrawTileJustified.
            "DrawTileJustified" => {
                let mat = args[0].clone();
                let (x, y) = (self.cf(c, "CurX"), self.cf(c, "CurY"));
                self.canvas_draw_tile_justified(c, &mat, x, y, f(2), f(3), args[1].as_byte());
                Value::Void
            }
            // UCanvas::execDrawLine.
            "DrawLine" => {
                let color = bgra(&args[4]);
                let style = if color[3] == 255 { 1 } else { 5 };
                self.util_line(f(0), f(1), f(2), f(3), color, style);
                Value::Void
            }
            // UCanvas::execWrapStringToArray.
            "WrapStringToArray" => {
                let text = args[0].as_name();
                let eol = if given(3) { args[3].as_name().chars().next().unwrap_or('\0') } else { '|' };
                let lines = self.canvas_wrap_string_to_array(c, &text, f(2), &Value::Object(None), eol);
                if args.len() > 1 {
                    // The lines are appended to the script's array.
                    let mut out = match &args[1] {
                        Value::Array(a) => a.clone(),
                        _ => Vec::new(),
                    };
                    out.extend(lines.into_iter().map(Value::Str));
                    args[1] = Value::Array(out);
                }
                Value::Void
            }
            // UCanvas::execDrawCineFrame.
            "DrawCineFrame" => {
                let on = args.first().is_some_and(|v| v.as_bool());
                let fl = self.canvas_state.frame_length;
                let delay = self.cf(c, "CineFrameDelay");
                let mut a = self.cf(c, "CineFrameAlpha");
                if !on {
                    if 0.0 < a {
                        a -= fl / delay;
                        if a < 0.0 {
                            a = 0.0;
                        }
                    }
                } else if a < 1.0 {
                    a = fl / delay + a;
                    if 1.0 < a {
                        a = 1.0;
                    }
                }
                self.set_cf(c, "CineFrameAlpha", a);
                if 0.0 < a {
                    let f1 = a * 0.09;
                    let f3 = a * 0.2;
                    self.canvas_draw_frame(c, f3, 1.0 - f3, f1, 1.0 - f1);
                }
                Value::Void
            }
            // UCanvas::execDrawMsgboxBackground.
            "DrawMsgboxBackground" => {
                let only = args.first().is_some_and(|v| v.as_bool());
                self.canvas_msgbox_background(c, only, f(1), f(2), f(3), f(4), f(5), f(6));
                Value::Void
            }
            // UCanvas::execDrawActor: (Actor, WireFrame, ClearZ, DisplayFOV); the actor receives
            // LastRenderTime (visibility.rs). The game's calls (Inventory.uc, Weapon.uc) pass WireFrame false,
            // ClearZ false or omitted and no FOV.
            "DrawActor" => {
                if let Some(a) = args.first().and_then(|v| v.as_object()) {
                    let wire = args.get(1).is_some_and(|v| v.as_bool());
                    let clear_z = given(2) && args[2].as_bool();
                    self.canvas_draw_actor(a, wire, clear_z, if given(3) { Some(f(3)) } else { None });
                }
                Value::Void
            }
            // UCanvas::execDrawPortal.
            "DrawPortal" => {
                let i = |k: usize| args.get(k).map(|v| v.as_int()).unwrap_or(0);
                let fov = if given(7) { i(7) } else { 90 };
                let clear_z = if given(8) { args[8].as_bool() } else { true };
                let actor = args.get(4).and_then(|v| v.as_object());
                self.canvas_draw_portal(c, [i(0), i(1), i(2), i(3)], actor, args[5].as_vector(), args[6].as_rotator(), fov, clear_z);
                Value::Void
            }
            _ => return None,
        })
    }

    /// Tile of a static HUD UFinalBlend (FB_AlphaBlend, ZWrite, ZTest, AlphaTest and
    /// TwoSided cleared) on `material`, colour DrawColor as is.
    #[allow(clippy::too_many_arguments)]
    fn hud_blend_tile(&mut self, c: ObjId, key: &str, x1: f32, y1: f32, x2: f32, y2: f32, uv: [f32; 4]) {
        let color = self.draw_color(c);
        let tile = self.util_tile(x1, y1, x2, y2, uv, key, color);
        self.hud.push(tile);
    }

    /// Rounded background of a line of the life block: left edge, middle stretched over one texel, right
    /// edge mirrored, at (CurX, CurY = YP + 4).
    fn player_info_background(&mut self, c: ObjId, hud: ObjId, key: &str, side: f32, middle: f32) {
        let y = self.cf(hud, "YP") + 4.0;
        self.set_cf(c, "CurY", y);
        let cx = self.cf(c, "CurX");
        self.hud_blend_tile(c, key, cx, y, side + cx, y + side, [1.0, 32.0, 31.0, 64.0]);
        let (cx, cy) = (self.cf(c, "CurX"), self.cf(c, "CurY"));
        self.hud_blend_tile(c, key, side + cx, cy, (middle + cx) + side, side + cy, [30.0, 47.0, 31.0, 48.0]);
        let two = side + side;
        let (cx, cy) = (self.cf(c, "CurX"), self.cf(c, "CurY"));
        self.hud_blend_tile(c, key, (middle + cx) + side, cy, (two + cx) + middle, side + cy, [31.0, 32.0, 1.0, 64.0]);
    }

    /// Icon of a line (IconSize × 32) at (CurX, CurY = YP).
    fn player_info_icon(&mut self, c: ObjId, hud: ObjId, key: &str, uv: [f32; 4]) {
        let yp = self.cf(hud, "YP");
        self.set_cf(c, "CurY", yp);
        let icon = self.cf(hud, "IconSize") * 32.0;
        let cx = self.cf(c, "CurX");
        self.hud_blend_tile(c, key, cx, yp, icon + cx, yp + icon, uv);
    }

    /// UCanvas::DrawPlayerInfo (vtable +0x7C): backgrounds and icons of the life block,
    /// waves and markers of the sixth sense, as FB_AlphaBlend of HudIcons.
    pub(crate) fn canvas_draw_player_info(&mut self, c: ObjId, hud: ObjId, info: &crate::hud_native::PlayerInfo) {
        let icons = Self::material_key(&self.prop(hud, "HudIcons"));
        let key = format!("#canvas:2:0:127|{icons}");
        let h = self.propr(hud, "LifeDisplayHeight").as_int();
        let side = (h - 4) as f32;
        let middle = (self.propr(hud, "LifeDisplayWidth").as_int() + 8) as f32;
        let yp = self.cf(hud, "YP") - h as f32;
        self.set_cf(hud, "YP", yp);
        let color = |vm: &Vm, n: &str| bgra(&vm.prop(hud, n));
        let warning = color(self, "HudWarningColor");
        let warning_bg = [warning[0], warning[1], warning[2], 0x5a];
        let set_color = |vm: &mut Vm, k: [u8; 4]| vm.set_prop(c, "DrawColor", color_value(k));
        // Health (warning below 30).
        let low = info.health < 30.0;
        set_color(self, if low { warning_bg } else { color(self, "HudBackGroundColor") });
        let xp = self.cf(hud, "XP");
        self.set_cf(c, "CurX", xp);
        self.player_info_background(c, hud, &key, side, middle);
        set_color(self, if low { warning } else { color(self, "WhiteColor") });
        self.player_info_icon(c, hud, &key, [0.0, 0.0, 32.0, 32.0]);
        if info.crouched {
            set_color(self, color(self, "WhiteColor"));
            let x = (middle + self.cf(hud, "XP")) + side;
            self.set_cf(c, "CurX", x);
            let icon = self.cf(hud, "IconSize") * 32.0;
            let cy = self.cf(c, "CurY");
            self.hud_blend_tile(c, &key, x, cy, x + icon, icon + cy, [96.0, 32.0, 128.0, 64.0]);
            let xp = self.cf(hud, "XP");
            self.set_cf(c, "CurX", xp);
        }
        let mut rows: Vec<(bool, [f32; 4])> = Vec::new();
        if info.vest {
            rows.push((false, [32.0, 0.0, 64.0, 32.0]));
        }
        if info.helm {
            rows.push((false, [64.0, 0.0, 96.0, 32.0]));
        }
        if info.breath {
            rows.push((info.breath_time < 10.0, [32.0, 32.0, 64.0, 64.0]));
        }
        if info.chrono {
            rows.push((self.pb(hud, "bDrawChronoWithWarningColors"), [64.0, 32.0, 96.0, 64.0]));
        }
        for (alert, uv) in rows {
            let yp = self.cf(hud, "YP") - h as f32;
            self.set_cf(hud, "YP", yp);
            set_color(self, if alert { warning_bg } else { color(self, "HudBackGroundColor") });
            self.player_info_background(c, hud, &key, side, middle);
            set_color(self, if alert { warning } else { color(self, "WhiteColor") });
            self.player_info_icon(c, hud, &key, uv);
        }
        if info.six_sense {
            set_color(self, color(self, "HudBasicColor"));
            let s = (h - 4) as f32;
            // Double constants: 0.6 and 0.7 in single precision.
            let (k06, k07) = (0.6f32 as f64, 0.7f32 as f64);
            let t1 = self.level_time_times(k06);
            let p1 = ((t1 - ftol(t1) as f32) * s) * 2.0;
            let t2 = (((self.level_time - k07) as f32) as f64 * k06) as f32;
            let mut p2 = ((t2 - ftol(t2) as f32) * s) * 2.0;
            // First wave: the value left on the stack (the original p2 if it exceeded p1).
            let first = if !(p2 > p1) { p1 } else {
                let original = p2;
                p2 = ftol(p1) as f32;
                original
            };
            let yp = self.cf(hud, "YP") - h as f32;
            self.set_cf(hud, "YP", yp);
            for p in [first, p2] {
                let alpha = ftol((1.0 - (p / s) * 0.5) * 255.0) as u8;
                let mut k = self.draw_color(c);
                k[3] = alpha;
                set_color(self, k);
                let half = 0.5 * p;
                let x = ((h - 4) as f32 + self.cf(hud, "XP")) - half;
                self.set_cf(c, "CurX", x);
                let y = (self.cf(hud, "YP") - half) + 4.0;
                self.set_cf(c, "CurY", y);
                self.hud_blend_tile(c, &key, x, y, p + x, y + p, [64.0, 64.0, 128.0, 128.0]);
            }
            let clip_x = self.cf(c, "ClipX");
            if let (Value::Array(mut timers), Value::Array(positions)) = (self.prop(hud, "fDrawSixSenseTimer"), self.prop(hud, "vSixSensePos")) {
                for i in 0..5.min(timers.len()) {
                    let timer = timers[i].as_float();
                    if timer <= 0.0 || timer.is_nan() {
                        continue;
                    }
                    let pos = positions.get(i).map(|p| p.as_vector()).unwrap_or([0.0; 3]);
                    let size = (timer + 1.0) * 0.5 * pos[2] * clip_x * 0.0015625 * 64.0;
                    let half = (0.5f64 * size as f64) as f32;
                    let x = pos[0] - half;
                    self.set_cf(c, "CurX", x);
                    set_color(self, [255; 4]);
                    let y = pos[1] - half;
                    self.set_cf(c, "CurY", y);
                    self.hud_blend_tile(c, &key, x, y, size + x, size + y, [0.0, 64.0, 64.0, 128.0]);
                    timers[i] = Value::Float(timer - 0.025);
                }
                self.set_prop(hud, "fDrawSixSenseTimer", Value::Array(timers));
            }
        }
    }

    /// UCanvas::DrawStdBackGround (vtable +0x78): rounded background (edges of `height`,
    /// middle of `center_width`) of HudWIcons as FB_AlphaBlend, at (CurX, CurY).
    pub(crate) fn canvas_draw_std_background(&mut self, c: ObjId, material: &Value, center_width: f32, height: f32) {
        let key = format!("#canvas:2:0:127|{}", Self::material_key(material));
        let (w, hh) = (center_width, height);
        let (cx, cy) = (self.cf(c, "CurX"), self.cf(c, "CurY"));
        self.hud_blend_tile(c, &key, cx, cy, hh + cx, hh + cy, [192.0, 224.0, 224.0, 256.0]);
        self.hud_blend_tile(c, &key, hh + cx, cy, (w + cx) + hh, hh + cy, [222.0, 240.0, 223.0, 241.0]);
        self.hud_blend_tile(c, &key, (w + cx) + hh, cy, ((hh + hh) + cx) + w, hh + cy, [224.0, 224.0, 192.0, 256.0]);
    }

    /// UCanvas::DrawWeaponList (vtable +0x80): one cell per weapon (red background without
    /// ammunition, orange for a two-handed weapon when only one hand is free), its icon taken
    /// from HudWIcons (4 per row, 64 × 32), a frame around the weapon in hand, then the two
    /// rounded ends of the row; FB_AlphaBlend of HudWIcons.
    pub(crate) fn canvas_draw_weapon_list(&mut self, c: ObjId, hud: ObjId, list: &[Option<ObjId>], count: i32, max_group: usize) {
        let key = format!("#canvas:2:0:127|{}", Self::material_key(&self.prop(hud, "HudWIcons")));
        let h = self.propr(hud, "LifeDisplayHeight").as_int();
        let right = self.cf(hud, "RightMargin");
        let mut x = ftol(((1.0 - right) * self.cf(c, "ClipX") - (((h + 2) * count) << 1) as f32) - (h / 2) as f32);
        let mut border = bgra(&self.prop(hud, "HudBasicColor"));
        border[3] = 0xff;
        self.set_prop(c, "BorderColor", color_value(border));
        let yp = self.cf(hud, "YP");
        self.set_cf(c, "CurY", yp);
        let drawn = self.propr(hud, "DrawnWeapon").as_object();
        let set = |vm: &mut Vm, n: &str| {
            let v = vm.prop(hud, n);
            vm.set_prop(c, "DrawColor", v);
        };
        for g in 0..=max_group {
            let Some(Some(w)) = list.get(g).copied() else { continue };
            let slave_loaded = self.propr(w, "bHaveSlave").as_bool()
                && self.propr(w, "MySlave").as_object().filter(|&s| self.objects.get(s).is_some_and(|o| o.is_some())).is_some_and(|s| self.propr(s, "ReloadCount").as_byte() > 0);
            if !self.weapon_has_ammo(w) && !self.weapon_has_alt_ammo(w) && !slave_loaded {
                set(self, "RedColor");
            } else {
                let pawn_one_hand = self.propr(hud, "PawnOwner").as_object().is_some_and(|p| self.propr(p, "bHaveOnlyOneHandFree").as_bool());
                if self.propr(w, "WHand").as_byte() == 2 && pawn_one_hand {
                    set(self, "OrangeColor");
                } else {
                    set(self, "HudBackGroundColor");
                }
            }
            let mut k = self.draw_color(c);
            k[3] = if Some(w) == drawn { 0x8c } else { 0x46 };
            self.set_prop(c, "DrawColor", color_value(k));
            let cx = x as f32;
            self.set_cf(c, "CurX", cx);
            let cy = self.cf(c, "CurY");
            self.hud_blend_tile(c, &key, cx, cy, (2 * h) as f32 + cx, h as f32 + cy, [222.0, 239.0, 223.0, 240.0]);
            set(self, "WhiteColor");
            let gi = g as i32;
            let row = gi / 4;
            let col = gi - row * 4;
            let (u1, v1) = (col * 64, row * 32);
            // Mods build: the HUD icon of a mod's new weapon instead of its slot's tile
            // (mod_weapons.rs).
            #[cfg(feature = "mods")]
            {
                let sheet = self.prop(hud, "HudWIcons");
                if let Some((icon, [u, v, ul, vl])) = self.mod_weapon_icon(w, &sheet) {
                    let icon_key = format!("#canvas:2:0:127|{}", Self::material_key(&icon));
                    self.hud_blend_tile(c, &icon_key, cx, cy, (2 * h) as f32 + cx, h as f32 + cy, [u, v, u + ul, v + vl]);
                } else {
                    self.hud_blend_tile(c, &key, cx, cy, (2 * h) as f32 + cx, h as f32 + cy, [u1 as f32, v1 as f32, (u1 + 64) as f32, (v1 + 32) as f32]);
                }
            }
            #[cfg(not(feature = "mods"))]
            self.hud_blend_tile(c, &key, cx, cy, (2 * h) as f32 + cx, h as f32 + cy, [u1 as f32, v1 as f32, (u1 + 64) as f32, (v1 + 32) as f32]);
            if Some(w) == drawn {
                let st = self.canvas_style(c);
                let bc = bgra(&self.prop(c, "BorderColor"));
                let mut a = cx - 0.5;
                let mut b = cy - 0.5;
                let mut cc = ((2 * h) as f32 + a) + 0.5;
                let mut d = (h as f32 + b) + 0.5;
                for pass in 0..2 {
                    if pass == 1 {
                        a -= 1.0;
                        b -= 1.0;
                        cc += 1.0;
                        d += 1.0;
                    }
                    self.util_line((a as f64 + 0.5) as f32, b, cc, b, bc, st);
                    self.util_line(a, d, (cc as f64 + 1.0) as f32, d, bc, st);
                    self.util_line(a, b, a, d, bc, st);
                    self.util_line(cc, b, cc, d, bc, st);
                }
            }
            x = x + 2 * h + 4;
        }
        let mut k = bgra(&self.prop(hud, "HudBackGroundColor"));
        k[3] = 0x46;
        self.set_prop(c, "DrawColor", color_value(k));
        let cx = x as f32;
        self.set_cf(c, "CurX", cx);
        let cy = self.cf(c, "CurY");
        self.hud_blend_tile(c, &key, cx, cy, h as f32 + cx, h as f32 + cy, [224.0, 224.0, 192.0, 256.0]);
        let left = ftol(((((1.0 - right) * self.cf(c, "ClipX") - (((h + 2) * count) << 1) as f32) as f64 - h as f64 * 1.5) as f32 as f64 - 4.0) as f32);
        let lx = left as f32;
        self.set_cf(c, "CurX", lx);
        self.hud_blend_tile(c, &key, lx, cy, h as f32 + lx, h as f32 + cy, [192.0, 224.0, 224.0, 256.0]);
    }

    /// UCanvas::execDrawPortal: black rectangle (FCanvasUtil::DrawTile without a material,
    /// Z = 1), SetViewport on the rectangle, depth cleared if `clear_z`, then
    /// FCameraSceneNode::Render of the given camera with the player's FovAngle = `fov`, viewport+0x120
    /// reset to 0 and +0x124 to 1: the scene, then HUD.PostRender (without the interactions afterwards) whose
    /// primitives are drawn in the rectangle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn canvas_draw_portal(&mut self, c: ObjId, rect: [i32; 4], actor: Option<ObjId>, location: [f32; 3], rotation: [i32; 3], fov: i32, clear_z: bool) {
        let [x, y, w, h] = rect;
        let tile = self.util_tile(x as f32, y as f32, (w + x) as f32, (h + y) as f32, [0.0; 4], "#white", [0, 0, 0, 255]);
        self.hud.push(tile);
        let rect = [x as f32, y as f32, w as f32, h as f32];
        let portal_index = self.hud.len();
        self.hud.push(HudDraw::Portal { rect, location, rotation, fov: fov as f32, clear_z, filter: None });
        let saved = self.only_post_render;
        self.only_post_render = false;
        let pc = self.render_player;
        // FCameraSceneNode (Viewport, Actor, Location, Rotation, FOV) then FCameraSceneNode::Render:
        // the portal's scene gives LastRenderTime to its actors (projection on
        // the W × H rectangle).
        let node = self.camera_scene(pc, actor, location, rotation, fov as f32, (w.max(1) as u32, h.max(1) as u32));
        // The portal's scene has its own translucent list (rebuilds it for each
        // scene): the viewport scene's stays that of its drawing.
        let saved_rendered = std::mem::take(&mut self.rendered);
        let saved_order = std::mem::take(&mut self.translucent_order);
        let saved_light_passes = std::mem::take(&mut self.display.scene_light_passes);
        let source_start = self.display.canvas_source_draws.len();
        self.render_scene(&node);
        let secondary_light_passes = std::mem::replace(&mut self.display.scene_light_passes, saved_light_passes);
        self.translucent_order = saved_order;
        let drawn = std::mem::replace(&mut self.rendered, saved_rendered);
        self.canvas_rendered.extend(drawn);
        self.display.portal_light_passes.insert(portal_index, secondary_light_passes);
        let first = self.hud.len();
        // FCameraSceneNode::Render of the portal, after the scene: Viewport+0x120 is zero
        // (DrawPortal has just reset it to 0), so as for the viewport, the frame shake and
        // the colour filter of Viewport->Actor, with the same Viewport+0x148 (no new
        // Lock): FilterColor advances once more per portal.
        if let Some(p) = pc.filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some())) {
            self.camera_shake_frame(p);
            let f = self.camera_filter(p, self.viewport_delta);
            if let Some(HudDraw::Portal { filter, .. }) = self.hud.get_mut(portal_index) {
                *filter = f;
            }
        }
        let saved_fov = pc.map(|p| self.prop(p, "FovAngle"));
        if let Some(p) = pc {
            self.set_prop(p, "FovAngle", Value::Float(fov as f32));
        }
        let fov_angle = pc.map_or(0.0, |p| self.pf(p, "FovAngle"));
        self.canvas_portal_calls.push(crate::PortalCall { rect: [x, y, w, h], actor, location, rotation, fov, clear_z, fov_angle });
        self.master_pre_render(c);
        if let Some(hud) = pc.and_then(|p| self.propr(p, "myHUD").as_object()).filter(|&h| self.objects[h].is_some()) {
            self.event_pub(hud, "PostRender", vec![Value::Object(Some(c))]);
        }
        // FCanvasUtil keeps the whole screen (SizeX, SizeY), which the D3D viewport brings down to the rectangle.
        let (sx, sy) = (rect[2] / self.viewport.0.max(1) as f32, rect[3] / self.viewport.1.max(1) as f32);
        let map = |p: [f32; 2]| [rect[0] + p[0] * sx, rect[1] + p[1] * sy];
        for d in &mut self.hud[first..] {
            match d {
                HudDraw::Tile { x1, y1, x2, y2, .. } => {
                    [*x1, *y1] = map([*x1, *y1]);
                    [*x2, *y2] = map([*x2, *y2]);
                }
                HudDraw::Triangle { p, .. } => *p = p.map(map),
                HudDraw::Line { from, to, .. } => {
                    *from = map(*from);
                    *to = map(*to);
                }
                HudDraw::Portal { .. } => {}
            }
        }
        if let (Some(p), Some(f)) = (pc, saved_fov) {
            self.set_prop(p, "FovAngle", f);
        }
        let sources: Vec<_> = self.display.canvas_source_draws.drain(source_start..).collect();
        if !sources.is_empty() {
            self.display.portal_light_passes.entry(portal_index).or_default().push(crate::visibility::SceneLightPass {
                lights:None,sky_zone: None, leaves: None, draws: sources,
            });
        }
        self.only_post_render = saved;
    }

    /// Node of URenderTargetMaterial::Update: view FTranslationMatrix(−CamLocation) ×
    /// FInverseRotationMatrix(CamRotation) × screen frame, projection on the
    /// rectangle. The half-angle is FOV × 0.5 × π (3.1415265) / 180,
    /// each operation rounded to single (24-bit x87), and not FCameraSceneNode's FOV × 0.5 × π/180:
    /// one ulp less is enough to bring an actor in or out at the edge
    /// of a comic window.
    pub(crate) fn render_target_node(&self, pc: Option<ObjId>, location: [f32; 3], rotation: [i32; 3], fov: f32, size: (u32, u32)) -> crate::visibility::SceneNode {
        let half = ((fov * 0.5) * f32::from_bits(0x4049_0EC5)) / 180.0;
        self.scene_node(pc, pc, location, rotation, fov, half, (size.0.max(1), size.1.max(1)))
    }

    /// URenderTargetMaterial::Update: FLevelSceneNode of the viewport (
    /// ViewActor = Viewport->Actor, +0x1CC), placed at CamLocation / CamRotation (zone from
    /// UModel::PointRegion; projection on the rectangle), then
    /// FLevelSceneNode::Render: the window's scene gives LastRenderTime to its
    /// actors, which are drawn into the texture.
    pub(crate) fn render_target_scene(&mut self, material: ObjId, location: [f32; 3], rotation: [i32; 3], fov: f32, size: (u32, u32)) {
        self.render_target_scenes += 1;
        // Viewport->Actor: the player of the current HUD, otherwise the first PlayerController (Update
        // is often called during the Tick).
        // Class flag (same answer as is_child_of of the PlayerController class).
        let pc = self.render_player.or_else(|| self.split_target_viewer(material)).or_else(|| {
            self.class("PlayerController")?;
            self.actors.iter().copied().find(|&a| self.is(a, crate::flags_classes::D::PlayerController))
        });
        let node = self.render_target_node(pc, location, rotation, fov, size);
        let saved_rendered = std::mem::take(&mut self.rendered);
        let saved_order = std::mem::take(&mut self.translucent_order);
        let saved_light_passes = std::mem::take(&mut self.display.scene_light_passes);
        let source_start = self.display.canvas_source_draws.len();
        self.render_scene(&node);
        let secondary_light_passes = std::mem::replace(&mut self.display.scene_light_passes, saved_light_passes);
        self.translucent_order = saved_order;
        let drawn = std::mem::replace(&mut self.rendered, saved_rendered);
        self.target_rendered.extend(drawn);
        let mut passes = secondary_light_passes;
        let sources: Vec<_> = self.display.canvas_source_draws.drain(source_start..).collect();
        if !sources.is_empty() {
            passes.push(crate::visibility::SceneLightPass { lights:None,sky_zone: None, leaves: None, draws: sources });
        }
        let live_serials: std::collections::HashSet<_> = self.render_views.values().flat_map(|v| v.iter().map(|v| v.serial)).collect();
        self.display.target_light_passes.retain(|serial, _| live_serials.contains(serial));
        self.display.target_light_passes.insert(self.render_serial, passes);
    }

    /// Local split screen (OpenXIII addition): each player's HUD has its own comic panel material
    /// (XIIIBaseHUD.CWndMat). Its Update during the Tick is seen by that HUD's player, as the single
    /// viewport's player sees it in solo: otherwise the first player would be the viewer and the
    /// other player's own body would be drawn in front of their full-screen panel camera
    /// (CWndFullScreen at the player's eye). None outside split screen: the engine's rule.
    fn split_target_viewer(&self, material: ObjId) -> Option<ObjId> {
        if !self.local_split() {
            return None;
        }
        self.actors.iter().copied().find(|&a| {
            self.is(a, crate::flags_classes::D::PlayerController)
                && self.po(a, "myHUD").is_some_and(|hud| self.objects.get(hud).is_some_and(|o| o.is_some()) && self.prop(hud, "CWndMat").as_object() == Some(material))
        })
    }

    /// One HUD frame: UCanvas::Update then HUD.PostRender(Canvas) for each player, as
    /// the engine does after the scene. The primitives are in `self.hud`.
    pub fn render_hud(&mut self, width: u32, height: u32) {
        self.render_hud_for(None, width, height);
    }

    /// HUD of a viewport (FCameraSceneNode::Render): that of its player `player` only;
    /// `None`: all PlayerControllers. Without a scene between the two (see `hud_begin`).
    pub fn render_hud_for(&mut self, player: Option<ObjId>, width: u32, height: u32) {
        if let Some(frame) = self.hud_begin(player, width, height) {
            let _ = self.hud_end(frame);
        }
    }

    /// Start of FCameraSceneNode::Render up to the scene: UCanvas::Update
    /// (Viewport->Canvas, vtable +0x70) then UInteractionMaster::MasterProcessPreRender,
    /// with Level.LevelAction = LEVACT_Precaching if UGameEngine::Draw is precaching.
    /// The scene (FLevelSceneNode::Render) and Apply2DEffect follow, then `hud_end`.
    pub fn hud_begin(&mut self, player: Option<ObjId>, width: u32, height: u32) -> Option<HudFrame> {
        self.hud.clear();
        self.hud_anchors.clear();
        self.hud_anchor_depth = None;
        self.display.portal_light_passes.clear();
        self.display.canvas_source_draws.clear();
        self.canvas_rendered.clear();
        self.canvas_actor_calls.clear();
        self.canvas_portal_calls.clear();
        // UViewport::SizeX/SizeY: the size of the drawn frame.
        self.viewport = (width as i32, height as i32);
        let canvas_class = self.class("Canvas")?;
        let canvas = match self.canvas {
            Some(c) if self.objects[c].is_some() => c,
            _ => {
                let c = self.spawn(canvas_class);
                // UCanvas::Init: fonts PoliceF16 and PoliceF20.
                for (prop, path) in [("SmallFont", "XIIIFonts.PoliceF16"), ("MedFont", "XIIIFonts.PoliceF20")] {
                    let v = if self.font(path).is_some() { Value::Asset(path.to_string()) } else { Value::Object(None) };
                    self.set_prop(c, prop, v);
                }
                self.canvas = Some(c);
                c
            }
        };
        self.canvas_update(canvas);
        let pc_class = self.class("PlayerController")?;
        // UGameEngine::Draw: PrecacheOnFlip and !bSuspendPrecaching → the frame is
        // drawn with Level.LevelAction = LEVACT_Precaching (4), restored afterwards, then
        // PrecacheOnFlip = 0. UsePrecaching comes from [D3DDrv.D3DRenderDevice].
        let precaching = std::mem::take(&mut self.precache_on_flip)
            && self
                .config
                .as_mut()
                .and_then(|c| c.get_string("D3DDrv.D3DRenderDevice", "UsePrecaching", "System"))
                .is_none_or(|v| v.eq_ignore_ascii_case("true") || v.trim() == "1");
        let saved_action = self.level.map(|l| self.prop(l, "LevelAction"));
        if let (true, Some(l)) = (precaching, self.level) {
            self.set_prop(l, "LevelAction", Value::Byte(4));
        }
        let players: Vec<ObjId> = match player {
            Some(p) => vec![p],
            None => {
                let all: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.objects[a].as_ref().is_some_and(|o| self.is_child_of(o.class, pc_class))).collect();
                // UGameEngine::Draw / FCameraSceneNode::Render: only
                // Viewport->Actor->myHUD is drawn. On a listen server, the controller of a
                // remote client (Player = UNetConnection) has no viewport: no PostRender.
                // Without any viewport (tests), every controller is kept.
                let local: Vec<ObjId> = all.iter().copied().filter(|&c| self.local_player_controller(c).is_some()).collect();
                if local.is_empty() { all } else { local }
            }
        };
        self.master_pre_render(canvas);
        Some(HudFrame { canvas, players, precaching, saved_action })
    }

    /// End of FCameraSceneNode::Render after the scene and Apply2DEffect (vtable +0x94):
    /// AHUD::eventPostRender of Viewport->Actor->myHUD, then MasterProcessPostRender.
    /// True if the frame was precaching and UGameEngine::Draw must draw the precache frame
    /// (`precache_wanted`).
    /// Controllers for which `hud_end` will call HUD.PostRender.
    pub fn hud_frame_players(frame: &HudFrame) -> &[ObjId] {
        &frame.players
    }

    pub fn hud_end(&mut self, frame: HudFrame) -> bool {
        let HudFrame { canvas, players, precaching, saved_action } = frame;
        self.reset_loading_origin(canvas);
        let scaled = self.hud_scale_begin(canvas);
        for pc in players {
            self.render_player = Some(pc);
            if let Some(hud) = self.propr(pc, "myHUD").as_object() {
                let show_flags = self.propr(pc, "ShowFlags").as_int() as u32;
                if self.objects[hud].is_some() && crate::camera_render::hud_post_render(false, show_flags, true) {
                    self.event_pub(hud, "PostRender", vec![Value::Object(Some(canvas))]);
                    // Mods build only: the tracers of weapons held with a FirstPersonModel, on
                    // the muzzle as the weapon is drawn (mod_weapons.rs).
                    #[cfg(feature = "mods")]
                    self.mod_tracer_fixup(canvas);
                    // Mods build only: what the scripts keep on this view (mod_script_hud.rs).
                    #[cfg(feature = "mods")]
                    if !self.mods.hud.items.is_empty() {
                        self.mod_hud_draw(pc, canvas);
                    }
                    self.coop_view_separator(pc, hud, canvas);
                }
            }
        }
        self.hud_scale_end(canvas, scaled);
        self.render_player = None;
        self.master_post_render(canvas);
        if let (true, Some(l), Some(a)) = (precaching, self.level, saved_action) {
            self.set_prop(l, "LevelAction", a);
        }
        precaching
    }

    /// UGameEngine::Draw: after a frame drawn as precache,
    /// PrecacheOnFlip reset to 0 and, unless Level.bNeverPrecache (+0x40C bit 0), a second
    /// FCameraSceneNode from the same viewpoint with Viewport+0x158 = 1 (`precache_render`) and
    /// LevelAction restored: full Render (HUD included), scene seen from zone 0 without a frustum,
    /// so all the actors of the display list are drawn
    /// (LastRenderTime, emitters' Inactive reset to 0 by RenderParticles).
    pub fn precache_wanted(&self, precaching: bool) -> bool {
        precaching && self.level.is_none_or(|l| !self.pb(l, "bNeverPrecache"))
    }
}

/// Centred 4:3 frame of a `clip_x` × `clip_y` canvas wider than 4:3: left offset and
/// width, in whole pixels; `None` in 4:3 or narrower (nothing changes).
pub(crate) fn frame_43(clip_x: f32, clip_y: f32) -> Option<(f32, f32)> {
    let width_value = (clip_y * 4.0 / 3.0).round();
    if !(clip_y > 0.0) || width_value >= clip_x {
        return None;
    }
    Some((((clip_x - width_value) * 0.5).floor(), width_value))
}

/// Fields XL / YL / DXS / DYS (640 × 480) of a comic panel, with the HUD's margins LeftMargin /
/// UpMargin, that make DrawWnd draw it as in the centred 4:3 frame
/// (`frame_43`): `None` if the canvas is not wider than 4:3 or if the panel, with its 8-pixel black
/// border, already fits in the height.
pub(crate) fn case_frame_43([xl, yl, dxs, dys]: [f32; 4], (margin_g, margin_h): (f32, f32), clip_x: f32, clip_y: f32) -> Option<[f32; 4]> {
    let (shift_amount, width_value) = frame_43(clip_x, clip_y)?;
    let (scale_value, scale_43) = (clip_x / 640.0, width_value / 640.0);
    if (yl + dys) * scale_value + margin_h * clip_y + 8.0 <= clip_y {
        return None;
    }
    // DrawWnd: XP = XL × ClipX / 640 + LeftMargin × ClipX, YP = YL × ClipX / 640 + UpMargin × ClipY.
    let k = scale_43 / scale_value;
    let xl_43 = (shift_amount + margin_g * (width_value - clip_x) + xl * scale_43) / scale_value;
    Some([xl_43, yl * k, dxs * k, dys * k])
}

#[cfg(test)]
mod frame_43_tests {
    use super::{frame_43, case_frame_43};

    #[test]
    fn case_frame_43_alarm() {
        // "ALARM!" panel of SPADS01 (Alarme2: 396, 264, 128 × 128) in a wide window.
        let (x, y) = (1920.0f32, 1129.0f32);
        let [xl, yl, dxs, dys] = case_frame_43([396.0, 264.0, 128.0, 128.0], (0.0, 0.0), x, y).unwrap();
        let s = x / 640.0;
        let (xp, yp, w, h) = (xl * s, yl * s, dxs * s, dys * s);
        assert!((xp - (207.0 + 396.0 * 1505.0 / 640.0)).abs() < 0.01, "{xp}");
        assert!(yp + h + 8.0 <= y && xp >= 0.0 && xp + w <= x, "{xp} {yp} {w} {h}");
        // HUD margins: the 4:3 frame applies them to its width.
        let [xl, ..] = case_frame_43([396.0, 264.0, 128.0, 128.0], (0.05, 0.05), x, y).unwrap();
        assert!((xl * s + 0.05 * x - (207.0 + 0.05 * 1505.0 + 396.0 * 1505.0 / 640.0)).abs() < 0.01);
        // Dialogue portrait or panel at the top: unchanged, it stays stuck to HudDlg's text.
        assert_eq!(case_frame_43([20.0, 20.0, 72.0, 72.0], (0.0, 0.0), x, y), None);
        assert_eq!(case_frame_43([0.0, 80.0, 160.0, 160.0], (0.0, 0.0), x, y), None);
        // 4:3: never touched.
        assert_eq!(case_frame_43([396.0, 264.0, 128.0, 128.0], (0.0, 0.0), 1600.0, 1200.0), None);
        assert_eq!(case_frame_43([396.0, 400.0, 128.0, 128.0], (0.0, 0.0), 640.0, 480.0), None);
    }

    #[test]
    fn frame_43_screens() {
        // 4:3 and narrower: unchanged, as in the original game.
        for (x, y) in [(640.0, 480.0), (800.0, 600.0), (1600.0, 1200.0), (1280.0, 1024.0), (800.0, 1200.0)] {
            assert_eq!(frame_43(x, y), None, "{x}x{y}");
        }
        // A 1920 × 1129 window and wide screens: centred frame of full height.
        assert_eq!(frame_43(1920.0, 1129.0), Some((207.0, 1505.0)));
        assert_eq!(frame_43(1920.0, 1080.0), Some((240.0, 1440.0)));
        assert_eq!(frame_43(1280.0, 720.0), Some((160.0, 960.0)));
        assert_eq!(frame_43(1920.0, 1200.0), Some((160.0, 1600.0)));
        assert_eq!(frame_43(2400.0, 1080.0), Some((480.0, 1440.0)));
        assert_eq!(frame_43(0.0, 0.0), None);
        // The bottom of a panel placed in 640 × 480 (Y + height ≤ 480) stays on the screen.
        for (x, y) in [(1920.0f32, 1129.0f32), (1920.0, 1080.0), (2560.0, 1080.0), (1366.0, 768.0), (2400.0, 1080.0)] {
            let (d, l) = frame_43(x, y).unwrap();
            assert!(480.0 * l / 640.0 <= y + 0.5, "{x}x{y}");
            assert!(d >= 0.0 && d + l <= x, "{x}x{y}");
        }
    }
}

