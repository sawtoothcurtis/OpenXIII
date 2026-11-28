//! DT_Sprite actors: sprite branch of FDynamicActor::Render
//! (Engine.dll). A quad facing the camera, of 128 × DrawScale × DrawScale3D (X as width, Y as
//! height), centered on Location, oriented by the scene node's CameraX and CameraY
//! (FCameraSceneNode +0x198 / +0x1A4, rows 0 and 1 of CameraToWorld: right and up).
//! Color FColor(FPlane(1, 1, 1, 1)) = 254 (FColor::FColor: fistp(X × 255 − 0.5)).
//!
//! Material: the actor's Texture. If it is a UTexture, a single static UFinalBlend
//! (created at the first sprite) receives it as Material, with TwoSided always set and
//! IgnoreFog = the actor's bIgnoreFog; depending on the texture and Style:
//! - bMasked or STY_Masked: FB_Overwrite, ZWrite, ZTest, AlphaTest, AlphaRef 127;
//! - bAlphaTexture: FB_AlphaBlend, without ZWrite, ZTest, AlphaTest, AlphaRef 0;
//! - STY_Translucent: FB_Translucent; STY_Modulated: FB_Modulate; without ZWrite, ZTest, without
//!   AlphaTest (AlphaRef unchanged);
//! - otherwise nothing changes: the UFinalBlend keeps the previous sprite's state (even from one frame to
//!   the next), as in the game.
//!
//! Another material class (Shader, FinalBlend…) is passed as is to SetMaterial.
//! Drawing: identity transform, DrawPrimitive(PT_TriangleFan, 2 triangles); sprites
//! go through the trails' path (TrailBatch.material: EnableLighting(0, 1), vertex
//! color, CM_None).
//!
//! Light coronas do not exist in XIII's engine: Actor has no bCorona,
//! no "Corona" string in Engine.dll or D3DDrv.dll, and ZoneInfo's LensFlares are
//! read by no native code. Nothing is therefore drawn for them.

use std::cell::RefCell;

use openxiii_vm::trail_render::{TrailBatch, TrailVertex};
use openxiii_vm::value::{ObjId, Value};
use openxiii_vm::Vm;

/// DT_Sprite.
const DT_SPRITE: u8 = 1;
/// STY_Masked, STY_Translucent, STY_Modulated.
const STY_MASKED: u8 = 2;
const STY_TRANSLUCENT: u8 = 3;
const STY_MODULATED: u8 = 4;
const SPRITE_SIZE: f32 = 128.0;
/// FColor(FPlane(1, 1, 1, 1)) as a D3DCOLOR.
const WHITE: u32 = 0xfefe_fefe;

/// State of the static UFinalBlend (FrameBufferBlending, ZWrite, ZTest, AlphaTest, AlphaRef);
/// the class's default values at its creation.
#[derive(Debug, Clone, Copy)]
struct Blend {
    fb: u8,
    z_write: bool,
    z_test: bool,
    alpha_test: bool,
    alpha_ref: u8,
}

impl Default for Blend {
    fn default() -> Self {
        Blend { fb: 0, z_write: true, z_test: true, alpha_test: false, alpha_ref: 0 }
    }
}

thread_local! {
    /// The shared UFinalBlend, which keeps its state from one sprite to the next.
    static BLEND: RefCell<Blend> = RefCell::new(Blend::default());
}

/// Sprites of the actors drawn in this frame, in the engine's visibility order.
/// `sky_value`: camera of the sky scene (FSkySceneNode) for an actor that only this scene draws;
/// its sprite faces this camera, turned by the SkyZoneInfo's Rotation (Bateau01's moon).
pub fn batches(vm: &mut Vm, camera: &crate::renderer::Camera, actors: &[ObjId], sky_value: &dyn Fn(ObjId) -> Option<crate::renderer::Camera>) -> Vec<TrailBatch> {
    let frame = camera.view_frame();
    let mut out = Vec::new();
    for &a in actors {
        if vm.objects.get(a).is_none_or(|o| o.is_none()) {
            continue;
        }
        // CameraX (right) and CameraY (up): rows 0 and 1 of CameraToWorld of the drawing scene.
        let (right, up) = match sky_value(a) {
            Some(c) => {
                let f = c.view_frame();
                (f.right, f.up)
            }
            None => (frame.right, frame.up),
        };
        if vm.native_prop(a, openxiii_vm::NativeField::DrawType).as_byte() != DT_SPRITE {
            continue;
        }
        let texture = match vm.prop(a, "Texture") {
            Value::Asset(p) if !p.is_empty() => p,
            _ => continue,
        };
        let style = vm.propr(a, "Style").as_byte();
        let ignore_fog = vm.propr(a, "bIgnoreFog").as_bool();
        let material = if is_texture(vm, &texture) {
            let masked = vm.asset_prop(&texture, "bMasked").is_some_and(|v| v.as_bool());
            let alpha = vm.asset_prop(&texture, "bAlphaTexture").is_some_and(|v| v.as_bool());
            let b = BLEND.with(|b| {
                let mut b = b.borrow_mut();
                if masked || style == STY_MASKED {
                    *b = Blend { fb: 0, z_write: true, z_test: true, alpha_test: true, alpha_ref: 0x7f };
                } else if alpha {
                    *b = Blend { fb: 2, z_write: false, z_test: true, alpha_test: true, alpha_ref: 0 };
                } else if style == STY_TRANSLUCENT || style == STY_MODULATED {
                    b.fb = if style == STY_TRANSLUCENT { 4 } else { 1 };
                    b.z_write = false;
                    b.z_test = true;
                    b.alpha_test = false;
                }
                *b
            });
            format!("#sprite:{}:{}:{}:{}:{}:{}|{texture}", b.fb, b.z_write as u8, b.z_test as u8, b.alpha_test as u8, b.alpha_ref, ignore_fog as u8)
        } else {
            // Other material: SetMaterial(Texture) as is.
            texture.clone()
        };
        let s = vm.propr(a, "DrawScale").as_float();
        let s3 = vm.propr(a, "DrawScale3D").as_vector();
        let l = vm.propr(a, "Location").as_vector();
        // A = CameraX × DrawScale × DrawScale3D.X × 128 × 0.5; B = −CameraY × DrawScale ×
        // DrawScale3D.Y × 128 × 0.5. Vertices: L − A − B, L + A − B, L + A + B, L − A + B.
        let ha = [0, 1, 2].map(|i| s * right[i] * s3[0] * SPRITE_SIZE * 0.5);
        let hb = [0, 1, 2].map(|i| -up[i] * s * s3[1] * SPRITE_SIZE * 0.5);
        let corner = |its: f32, sb: f32| [0, 1, 2].map(|i| l[i] + its * ha[i] + sb * hb[i]);
        let corners = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        // Fix that is not in the original game (improved build): fade and intensity of the translucent
        // halos (openxiii_vm::effects_angle::halos); a halo that is off is not sent.
        let mut color = WHITE;
        if style == STY_TRANSLUCENT && material.starts_with("#sprite:") {
            if let Some(k) = vm.halo_intensity(a, frame.location, frame.forward) {
                if k <= 0.0 {
                    continue;
                }
                let c = (254.0 * k.min(1.0)).round() as u32;
                color = c * 0x0101_0101;
            }
        }
        let vertices = corners.iter().zip(uvs).map(|(p, uv)| TrailVertex { position: *p, color, uv }).collect();
        out.push(TrailBatch {
            texture: Some(texture),
            frame_buffer_blending: 0,
            vertices,
            // PT_TriangleFan of two triangles.
            indices: vec![0, 1, 2, 0, 2, 3],
            lines: false,
            material: Some(material),
            delayed: vm.draw_list(a).0,
            ignore_vignette: vm.draw_list(a).1,
            actor: a,
        });
    }
    out
}

/// UObject::IsA(UTexture): the object's class is Texture or derives from it.
fn is_texture(vm: &mut Vm, path: &str) -> bool {
    let class = vm.library.as_mut().and_then(|lib| lib.resolve(path).map(|(loaded, index)| loaded.package.export_class(index).to_string()));
    match (class.and_then(|c| vm.class(&c)), vm.class("Texture")) {
        (Some(c), Some(t)) => vm.is_child_of(c, t),
        _ => false,
    }
}
