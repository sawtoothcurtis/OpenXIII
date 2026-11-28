/// Texels per unit of a render texture (256 units a side, TARGET_SIDE). Faithful build: 1, as
/// the original (pixelated comic panels). Improved build: the smallest power of two that gives
/// a panel at least the definition of a 480-line frame scaled to the world as drawn (720p 2,
/// 1080p and 1440p 4, 2160p 8), at most 8 on PC (2048 x 2048) and 4 on Android, within the
/// device's largest texture.
fn target_scale(world_h: u32, max_texture: u32) -> u32 {
    let cap = if cfg!(target_os = "android") { 4 } else { 8 };
    let mut scale = 1;
    while scale * 480 < world_h && scale * 2 <= cap && TARGET_SIDE as u32 * scale * 2 <= max_texture {
        scale *= 2;
    }
    scale
}

/// Side of the scene texture of the 2D screen effects (blur, sharpen, poison, injury, water,
/// flash). Faithful build: 512, as D3DDrv. Improved build: the smallest power of
/// two from 512 that is at least the world's height as drawn (720p 1024, 1080p and above 2048),
/// at most 2048 on PC and 1024 on Android, within the device's largest texture.
fn fx_scene_side(world_h: u32, max_texture: u32) -> u32 {
    let mut side = crate::screen_fx::SIDE;
    let cap = if cfg!(target_os = "android") { 1024 } else { 2048 };
    while side < world_h && side * 2 <= cap.min(max_texture) {
        side *= 2;
    }
    side
}

/// Side of the shadow targets for a world drawn `world_h` pixels high (render scale included).
/// Faithful build: 256, as D3DDrv. Improved build: the smallest power of two from 256 that is at
/// least half the world's height (720p 512, 1080p and 1440p 1024, 2160p 2048), at most 2048 on
/// PC and 1024 on Android, and never beyond the device's largest texture. Eight targets of
/// 1024 × 1024 take 64 MB (colour and depth), of 2048 × 2048 256 MB.
/// quality is the Video menu's choice: 0 follows the height as above, another value is the
/// side itself (256, 1024 or 2048), under the same limits.
fn shadow_target_size(world_h: u32, max_texture: u32, quality: u32) -> u32 {
    let cap = if cfg!(target_os = "android") { 1024 } else { 2048 };
    let cap = cap.min(max_texture.max(SHADOW_TARGET_SIZE));
    if quality != 0 {
        return quality.clamp(SHADOW_TARGET_SIZE, cap);
    }
    let mut size = SHADOW_TARGET_SIZE;
    while size < world_h / 2 && size * 2 <= cap {
        size *= 2;
    }
    size
}

#[cfg(test)]
#[test]
fn render_textures_follow_the_world_height_only_in_the_improved_build() {
    let sides: Vec<u32> = [480, 512, 720, 1080, 2160].iter().map(|&h| fx_scene_side(h, 16384)).collect();
    if !cfg!(target_os = "android") {
        assert_eq!(sides, [512, 512, 1024, 2048, 2048]);
        assert_eq!(fx_scene_side(2160, 1024), 1024);
    } else {
        assert_eq!(sides, [512, 512, 1024, 1024, 1024]);
    }
    let scales: Vec<u32> = [480, 600, 720, 1080, 1440, 2160, 4320].iter().map(|&h| target_scale(h, 16384)).collect();
    if !cfg!(target_os = "android") {
        assert_eq!(scales, [1, 2, 2, 4, 4, 8, 8]);
        assert_eq!(target_scale(2160, 1024), 4);
    } else {
        assert_eq!(scales, [1, 2, 2, 4, 4, 4, 4]);
    }
}

#[cfg(test)]
#[test]
fn shadow_targets_follow_the_world_height_only_in_the_improved_build() {
    let sizes: Vec<u32> = [480, 600, 720, 1080, 1440, 1620, 2160, 3240, 8640].iter().map(|&h| shadow_target_size(h, 16384, 0)).collect();
    if !cfg!(target_os = "android") {
        assert_eq!(sizes, [256, 512, 512, 1024, 1024, 1024, 2048, 2048, 2048]);
        assert_eq!(shadow_target_size(2160, 1024, 0), 1024);
        assert_eq!([256, 1024, 2048, 4096].map(|q| shadow_target_size(480, 16384, q)), [256, 1024, 2048, 2048]);
        assert_eq!(shadow_target_size(2160, 16384, 256), 256);
    } else {
        assert_eq!(sizes, [256, 512, 512, 1024, 1024, 1024, 1024, 1024, 1024]);
    }
}

fn held_frame_usage() -> wgpu::TextureUsages {
    wgpu::TextureUsages::COPY_DST
}

fn prepare_canvas_textures() {
    crate::onomatopoeias::kick();
}

fn resolve_canvas_textures(pass: &mut openxiii_assets::material::Pass) {
    if crate::onomatopoeias::active() {
        for st in pass.stages.iter_mut() {
            if let Some(TexRef::Bitmap(p)) = &st.texture {
                if let Some(key) = crate::onomatopoeias::master_for(p) {
                    st.texture = Some(TexRef::Bitmap(key));
                }
            }
        }
    }
}

fn sample_enlarged_text(point: bool, glyph: bool, enlarged: bool, graphics: openxiii_vm::video_settings::SessionGraphics) -> bool {
    openxiii_vm::gui::reads_bicubic(point, glyph, enlarged, openxiii_vm::gui::enlarged_text_bicubic(graphics))
}

fn set_canvas_filter(uniform: &mut PassUniform, pass: &openxiii_assets::material::Pass, point: bool, text_bicubic: bool) {
    if text_bicubic || (!point && matches!(&pass.stages[0].texture, Some(TexRef::Bitmap(p)) if openxiii_assets::mod_textures::is_fine(p))) {
        uniform.light_count[1] = 1;
    }
}

impl Renderer {
    /// Improved build: every menu onomatopoeia prepared, at each of its final sizes on the menu's
    /// pages (onomatopoeias::SIZES) times the menu's scale, before the canvas draws: at the first
    /// menu frame (after the startup's redrawing) and when the scale changes (resolution). The
    /// sizes no longer used are dropped with the groups that bound them.
    #[allow(dead_code)]
    fn prepare_onomatopoeias(&mut self, library: &mut Library) {
        let scale = self.onomatopoeia_scale;
        if scale <= 0.0 || scale == self.onomatopoeia_prepared_scale || !crate::onomatopoeias::active() {
            return;
        }
        let mut prepared: TableFast<String, Vec<(u32, u32, String)>> = Default::default();
        for &(name, mw, mh) in crate::onomatopoeias::SIZES {
            let Some(base) = crate::onomatopoeias::base(name) else { continue };
            let (w, h) = (((mw as f32) * scale).round().max(1.0) as u32, ((mh as f32) * scale).round().max(1.0) as u32);
            let key = format!("#ono:{}:{w}x{h}", name.to_ascii_lowercase());
            if !self.textures.contains_key(&key) {
                let mips = crate::onomatopoeias::at_size(&base, w as usize, h as usize);
                let view = upload(&self.device, &self.queue, &mips);
                self.textures.insert(key.clone(), Some(view));
            }
            let list = prepared.entry(name.to_ascii_lowercase()).or_default();
            if !list.iter().any(|e| e.2 == key) {
                list.push((w, h, key));
            }
        }
        let old: Vec<String> =
            self.onomatopoeia_prepared.values().flatten().map(|e| e.2.clone()).filter(|k| !prepared.values().flatten().any(|e| e.2 == *k)).collect();
        for k in old {
            self.drop_texture(library, &k);
        }
        self.onomatopoeia_prepared = prepared;
        self.onomatopoeia_prepared_scale = scale;
    }

    /// Improved build: the prepared texture an onomatopoeia drawn now reads: the smallest of its
    /// prepared sizes that covers the size it is drawn at (its final size once its pop-in zoom
    /// ends; smaller in between, reduced by the card), else its largest. Never computed here.
    #[allow(dead_code)]
    fn onomatopoeia_for(&self, path: &str) -> Option<String> {
        if !crate::onomatopoeias::is_onomatopoeia(path) {
            return None;
        }
        let name = path.rsplit('.').next()?.to_ascii_lowercase();
        let list = self.onomatopoeia_prepared.get(&name)?;
        let [sw, sh] = openxiii_vm::menu_figures::shown_size(path).unwrap_or([0.0, 0.0]);
        let fits = list.iter().filter(|e| e.0 as f32 + 0.5 >= sw && e.1 as f32 + 0.5 >= sh).min_by_key(|e| e.0 * e.1);
        fits.or_else(|| list.iter().max_by_key(|e| e.0 * e.1)).map(|e| e.2.clone())
    }

}

fn configured_shadow_target_size(world_h: u32, max_texture: u32, quality: u32) -> u32 {
    if openxiii_vm::video_settings::graphics_enhanced_local() {
        shadow_target_size(world_h, max_texture, quality)
    } else {
        SHADOW_TARGET_SIZE
    }
}

struct RenderTargetSizes {
    heat: bool,
    enhanced: bool,
}

impl RenderTargetSizes {
    fn local(renderer: &Renderer) -> Self {
        Self { enhanced: openxiii_vm::video_settings::graphics_enhanced_local(), heat: renderer.mod_surfaces.heat_used() }
    }

    fn panel_scale(&self, world_h: u32, max_texture: u32) -> u32 {
        if self.enhanced {
            target_scale(world_h, max_texture)
        } else {
            1
        }
    }

    fn effects_side(&self, world_h: u32, max_texture: u32) -> u32 {
        if self.enhanced || self.heat {
            fx_scene_side(world_h, max_texture)
        } else {
            crate::screen_fx::SIDE
        }
    }
}

/// Brightness, contrast and gamma in a window too (gamma.rs): Graphics Enhanced only; Graphics
/// Original keeps the original's ramp, full screen only.
pub(crate) fn gamma_in_window(graphics: openxiii_vm::video_settings::SessionGraphics) -> bool {
    graphics.enhanced()
}
