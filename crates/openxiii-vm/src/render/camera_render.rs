//! FCameraSceneNode::Render: sequence of calls for a viewport's frame and decisions, excluding drawing.
//!
//! In order: start of the FRenderInterface, black clear if the viewport is in wireframe (outside the editor), UCanvas::Update,
//! MasterProcessPreRender (the viewport has an InteractionMaster, +0x54), aspect mode (+0x11C: clear, UpdateMatrices and
//! SetViewport black bars), the scene (FLevelSceneNode::Render, except SetOnlyPostRender +0x120) followed by the frame shake, the
//! screen flash, the colour filter and Apply2DEffect, then, outside the editor and if ShowFlags has 8, the HUD (PostRender) between a
//! start and an end of the FRenderInterface, MasterProcessPostRender (except +0x124, set to 1 by DrawPortal) and the end.
//! FStats::Render ("stat" instruments) changes nothing in the game: outside `events`.

use crate::filter::{screen_flash_tile, shake_offsets};

/// What FCameraSceneNode::Render reads.
#[derive(Debug, Clone)]
pub struct CameraRender {
    /// GIsEditor (Core).
    pub editor: bool,
    /// UViewport::IsWire.
    pub wire: bool,
    /// UViewport+0x54: InteractionMaster.
    pub master: bool,
    /// UViewport+0x11C (zero as long as nothing writes it in game: only UViewport::UViewport sets it).
    pub aspect: bool,
    /// UViewport+0x120 (SetOnlyPostRender).
    pub only_post: bool,
    /// UViewport+0x124 (DrawPortal sets it to 1).
    pub no_post: bool,
    /// PlayerController.ShowFlags.
    pub show_flags: u32,
    /// PlayerController.myHUD (+0x42C) non-null.
    pub hud: bool,
    /// UViewport SizeX, SizeY (+0x88, +0x8C) and aspect ratio (+0x128).
    pub width: i32,
    pub height: i32,
    pub ratio: f32,
    /// Frame shake: (b, b + 1, a, a + 1), arguments of UCanvas::DrawFrame (`shake_offsets`).
    pub shake: Option<[f32; 4]>,
    /// Screen flash (B, G, R, A) (`screen_flash_tile`).
    pub flash: Option<[u8; 4]>,
    /// Colour filter (B, G, R, A) (`Vm::camera_filter`).
    pub filter: Option<[u8; 4]>,
}

/// Decisions used by the game: what is drawn around the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraPlan {
    /// FLevelSceneNode::Render, the shake, the flash and the filter.
    pub scene: bool,
    /// UInteractionMaster::MasterProcessPreRender.
    pub master_pre: bool,
    /// AHUD::eventPostRender of Viewport->Actor.myHUD.
    pub hud_post: bool,
    /// UInteractionMaster::MasterProcessPostRender.
    pub master_post: bool,
}

/// Coordinates passed to FRenderInterface::SetViewport for the black bars (UViewport+0x11C): (0, y, width, height); 24-bit
/// x87 (each operation rounded to f32), integer conversions by truncation (_ftol).
pub fn letterbox_viewport(width: i32, height: i32, ratio: f32) -> [i32; 4] {
    let h = height as f32;
    let q = h / ratio;
    [0, ((h - q) * 0.5) as i64 as i32, width, q as i64 as i32]
}

/// The player's HUD is drawn: outside the editor, ShowFlags has 8 (set to 0x30CC80C by SpawnPlayActor) and
/// the PlayerController has a HUD (+0x42C).
pub fn hud_post_render(editor: bool, show_flags: u32, hud: bool) -> bool {
    !editor && show_flags & 8 != 0 && hud
}

impl CameraRender {
    pub fn plan(&self) -> CameraPlan {
        CameraPlan { scene: !self.only_post, master_pre: self.master, hud_post: hud_post_render(self.editor, self.show_flags, self.hud), master_post: self.master && !self.no_post }
    }

    /// Sequence of calls: B start, C(clear), U Canvas::Update, P/O MasterProcessPre/PostRender(canvas), M
    /// UpdateMatrices, V(SetViewport), L(scene), F(DrawFrame), D(label, colour) DrawTile (fl: flash, fi: filter), A Apply2DEffect,
    /// H(HUD PostRender), E end. Address arguments are written cv (viewport canvas) and ri (FRenderInterface).
    pub fn events(&self) -> Vec<String> {
        let plan = self.plan();
        let mut v: Vec<String> = vec!["B".into()];
        if !self.editor && self.wire {
            v.push("C(1,ff000000,1,3f800000,1,0)".into());
        }
        v.push("U".into());
        if plan.master_pre {
            v.push("P(cv)".into());
        }
        let bars = |v: &mut Vec<String>, r: [i32; 4]| v.push(format!("V({:x},{:x},{:x},{:x})", r[0], r[1], r[2], r[3]));
        if self.aspect {
            v.push("C(1,ff000000,0,0,0,0)".into());
            v.push("M".into());
            bars(&mut v, letterbox_viewport(self.width, self.height, self.ratio));
        }
        if plan.scene {
            v.push("L(ri)".into());
            if self.aspect {
                v.push("M".into());
                bars(&mut v, [0, 0, self.width, self.height]);
            }
            if let Some(f) = self.shake {
                v.push(format!("F({:x},{:x},{:x},{:x})", f[0].to_bits(), f[1].to_bits(), f[2].to_bits(), f[3].to_bits()));
            }
            for (name_value, t) in [("fl", self.flash), ("fi", self.filter)] {
                if let Some(c) = t {
                    v.push(format!("D({name_value},{:08x})", u32::from_le_bytes(c)));
                }
            }
            v.push("A".into());
        }
        if !self.editor && self.show_flags & 8 != 0 {
            v.push("B".into());
            if self.hud {
                v.push("H(cv)".into());
            }
            v.push("E".into());
        }
        if plan.master_post {
            v.push("O(cv)".into());
        }
        v.push("E".into());
        v
    }

    /// Shake, flash and filter from the given fields.
    pub fn with_effects(mut self, shake: Option<(f32, [f32; 3], f32)>, flash: (f32, [f32; 3]), filter_color: [u8; 4]) -> Self {
        self.shake = shake.and_then(|(t, max, now)| shake_offsets(t, max, now)).map(|(a, b)| [b, b + 1.0, a, a + 1.0]);
        self.flash = screen_flash_tile(flash.0, flash.1);
        self.filter = crate::filter::filter_tile(filter_color);
        self
    }
}

