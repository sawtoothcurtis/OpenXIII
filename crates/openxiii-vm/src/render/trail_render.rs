//! Drawing of trails: ATrail::Render, called by FDynamicActor::Render for the
//! DT_Trail actors that the engine's visibility kept. The vertices come from the buffer of
//! trail.rs (FTrailVertexStream); FTrailVertexStream::GetStreamData ORs the
//! stream colour into each vertex's colour (the vertex's alpha, RibbonColor's RGB).

use crate::value::{ObjId, Value};
use crate::Vm;

/// Vertex sent to rendering: position (world), D3DCOLOR, U, V.
#[derive(Debug, Clone, Copy)]
pub struct TrailVertex {
    pub position: [f32; 3],
    pub color: u32,
    pub uv: [f32; 2],
}

/// One DrawPrimitive of ATrail::Render with the trail's UFinalBlend: Material (Texture, or
/// GEngine->WhiteTextureMaterial if `texture` is empty) and FrameBufferBlending. No lighting,
/// vertex colour (EnableLighting(0, 1)), no face culling (SetCullMode(CM_None)),
/// identity transform.
#[derive(Debug, Clone)]
pub struct TrailBatch {
    pub texture: Option<String>,
    pub frame_buffer_blending: u8,
    pub vertices: Vec<TrailVertex>,
    /// Triangles (PT_TriangleStrip) or segments (outline lines, PT_LineStrip) as a list.
    pub indices: Vec<u32>,
    pub lines: bool,
    /// Full material key instead of "#trail:…" (other primitives on the same path,
    /// like sprites).
    pub material: Option<String>,
    /// The actor's Vm::draw_list: deferred (1, 2 with bSpecialDelayFov) and bIgnoreVignetteAlpha.
    pub delayed: u8,
    pub ignore_vignette: bool,
    /// Drawn actor (rank in the translucent list, Vm::translucent_order).
    pub actor: ObjId,
}

/// XIII Color (fields B, G, R, A) as D3DCOLOR, as the machine code reads it (*(DWORD*)&Color).
fn d3dcolor(v: &Value) -> u32 {
    match v {
        Value::Struct(_, f) if f.len() == 4 => u32::from_le_bytes([f[0].as_byte(), f[1].as_byte(), f[2].as_byte(), f[3].as_byte()]),
        _ => 0,
    }
}

impl Vm {
    /// ATrail::Render of each trail drawn in this frame (`drawn`: actors kept by the
    /// engine's visibility).
    pub fn trail_frame(&self, drawn: &std::collections::HashSet<ObjId>) -> Vec<TrailBatch> {
        let mut actors: Vec<ObjId> = self.trails.keys().copied().filter(|a| drawn.contains(a) && self.alive(*a)).collect();
        actors.sort_unstable();
        let mut out = Vec::new();
        for a in actors {
            let Some(st) = self.trails.get(&a) else { continue };
            // Nothing below two sections.
            if st.count < 2 {
                continue;
            }
            let texture = match self.prop(a, "Texture") {
                Value::Asset(p) if !p.is_empty() => Some(p),
                _ => None,
            };
            // FrameBufferBlending: 4 (FB_Translucent) for STY_Translucent, 2 (FB_AlphaBlend)
            // for STY_Alpha, 4 otherwise.
            let fb = if self.propr(a, "Style").as_byte() == 5 { 2 } else { 4 };
            let (delayed, ignore_vignette) = self.draw_list(a);
            let (start, count, max) = (st.start as usize, st.count as usize, st.max as usize);
            let cross = self.pb(a, "CrossMode");
            // Ribbon: 2 × Count vertices from 2 × Start (2 × (Max + Start) for the second in
            // CrossMode), 2 × Count − 2 triangles as a strip.
            let ribbon = d3dcolor(&self.prop(a, "RibbonColor"));
            let mut firsts = vec![start * 2];
            if cross {
                firsts.push((max + start) * 2);
            }
            for first in firsts {
                let Some(verts) = st.verts.get(first..first + count * 2) else { continue };
                let vertices: Vec<TrailVertex> = verts.iter().map(|v| TrailVertex { position: v.pos, color: v.color | ribbon, uv: [v.u, v.v] }).collect();
                let indices: Vec<u32> = (0..(count * 2 - 2) as u32).flat_map(|k| [k, k + 1, k + 2]).collect();
                out.push(TrailBatch {
                    texture: texture.clone(),
                    frame_buffer_blending: fb,
                    vertices,
                    indices,
                    lines: false,
                    material: None,
                    delayed,
                    ignore_vignette,
                    actor: a,
                });
            }
            // Outline (DrawOutline): FB_AlphaBlend, OutlineColor, every other vertex (stride
            // 2), Count − 1 segments from Start then Start + 1 (2 × (Max + Start) and the next one
            // for the second ribbon); these starts are those of the original code.
            if self.pb(a, "DrawOutline") {
                let outline = d3dcolor(&self.prop(a, "OutlineColor"));
                let mut firsts = vec![start, start + 1];
                if cross {
                    firsts.push((max + start) * 2);
                    firsts.push((max + start) * 2 + 1);
                }
                for first in firsts {
                    let vertices: Vec<TrailVertex> = (0..count)
                        .filter_map(|k| st.verts.get(first + 2 * k))
                        .map(|v| TrailVertex { position: v.pos, color: v.color | outline, uv: [v.u, v.v] })
                        .collect();
                    if vertices.len() < 2 {
                        continue;
                    }
                    let indices: Vec<u32> = (0..vertices.len() as u32 - 1).flat_map(|k| [k, k + 1]).collect();
                    out.push(TrailBatch {
                        texture: texture.clone(),
                        frame_buffer_blending: 2,
                        vertices,
                        indices,
                        lines: true,
                        material: None,
                        delayed,
                        ignore_vignette,
                        actor: a,
                    });
                }
            }
        }
        out
    }
}
