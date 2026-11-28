//! Emulation of Direct3D 8's fixed pipeline as D3DDrv.dll configures it: render states
//! (D3DRS_*), texture stages (D3DTSS_COLOROP… up to RESULTARG), generation and
//! transformation of texture coordinates, basic lighting, alpha test and per-pixel linear
//! fog (D3DRS_FOGTABLEMODE = D3DFOG_LINEAR, set by UD3DRenderDevice::SetRes).
//!
//! Values come as is from `openxiii_assets::material` (SetMaterial passes);
//! the shader applies the D3DTOP/D3DTA operations with each stage's saturation.

use bytemuck::{Pod, Zeroable};
use openxiii_assets::material::{Pass, Stage, STAGES};

pub const SHADER: &str = r#"
override STAGE_COUNT: u32 = 5u;
// Specialized combiner (like the small program the Direct3D 8 driver prepares for each setting
// of the stages): SPECIAL true, the operations and arguments of each stage are constants of the
// pipeline (same values as in pu.st, same computation); false, they are read from the uniforms.
override SPECIAL: bool = false;
// Alpha to coverage (multisampled opaque pass with alpha test): the alpha written becomes the
// pixel's coverage around the reference, instead of the hard cut.
override A2C: bool = false;
override OPS0_C: u32 = 0u;
override OPS0_A: u32 = 0u;
override OPS0_W: u32 = 0u;
override OPS0_C0: u32 = 0u;
override OPS0_C1: u32 = 0u;
override OPS0_C2: u32 = 0u;
override OPS0_A0: u32 = 0u;
override OPS0_A1: u32 = 0u;
override OPS0_A2: u32 = 0u;
override OPS1_C: u32 = 0u;
override OPS1_A: u32 = 0u;
override OPS1_W: u32 = 0u;
override OPS1_C0: u32 = 0u;
override OPS1_C1: u32 = 0u;
override OPS1_C2: u32 = 0u;
override OPS1_A0: u32 = 0u;
override OPS1_A1: u32 = 0u;
override OPS1_A2: u32 = 0u;
override OPS2_C: u32 = 0u;
override OPS2_A: u32 = 0u;
override OPS2_W: u32 = 0u;
override OPS2_C0: u32 = 0u;
override OPS2_C1: u32 = 0u;
override OPS2_C2: u32 = 0u;
override OPS2_A0: u32 = 0u;
override OPS2_A1: u32 = 0u;
override OPS2_A2: u32 = 0u;
override OPS3_C: u32 = 0u;
override OPS3_A: u32 = 0u;
override OPS3_W: u32 = 0u;
override OPS3_C0: u32 = 0u;
override OPS3_C1: u32 = 0u;
override OPS3_C2: u32 = 0u;
override OPS3_A0: u32 = 0u;
override OPS3_A1: u32 = 0u;
override OPS3_A2: u32 = 0u;
override OPS4_C: u32 = 0u;
override OPS4_A: u32 = 0u;
override OPS4_W: u32 = 0u;
override OPS4_C0: u32 = 0u;
override OPS4_C1: u32 = 0u;
override OPS4_C2: u32 = 0u;
override OPS4_A0: u32 = 0u;
override OPS4_A1: u32 = 0u;
override OPS4_A2: u32 = 0u;
struct Frame {
    view_proj: mat4x4<f32>,
    // Direct3D view matrix (world -> camera), by rows: v' = M * v with this layout.
    view: mat4x4<f32>,
    // x: fog on, y: start, z: end.
    fog: vec4<f32>,
    fog_color: vec4<f32>,
    // D3DRS_AMBIENT.
    ambient: vec4<f32>,
};
@group(0) @binding(0) var<uniform> fr: Frame;

struct StageU {
    // color_op, alpha_op, tex_coord_index, transform_flags (0 without a matrix)
    ops: vec4<u32>,
    // color_arg0, color_arg1, color_arg2, result_arg
    cargs: vec4<u32>,
    // alpha_arg0, alpha_arg1, alpha_arg2, 0
    aargs: vec4<u32>,
    m: mat4x4<f32>,
};
struct PassU {
    st: array<StageU, 5>,
    tfactor: vec4<f32>,
    // x: stages used, y: alpha test, z: alpha reference, w: bits (1: lighting,
    // 2: colorvertex, 4: color in the stream, 8: fog ignored, 16: own fog
    // color, 32: local normals, 64: forced diffuse (c4), 128: outline (vertices pushed out),
    // 256: normal = local position, 512: position already in screen coordinates (Canvas))
    flags: vec4<u32>,
    fog_color: vec4<f32>,
    // D3DRS_AMBIENT.
    ambient: vec4<f32>,
    // Constants of D3DDrv's cartoon shaders: c4 (color, outline width in w), c5, c6.
    c4: vec4<f32>,
    c5: vec4<f32>,
    c6: vec4<f32>,
    // The actor's LocalToWorld (rows) and its inverse.
    l2w: mat4x4<f32>,
    w2l: mat4x4<f32>,
    // D3DLIGHT8: position and type (1 point, 2 spot, 3 directional), direction and
    // range, diffuse, (Attenuation1, cos(Phi / 2), 0, 0).
    lights: array<vec4<f32>, 32>,
    light_count: vec4<u32>,
    // Fog of the zone of the drawn element (SetDistanceFog per zone): x 0 = the
    // frame's fog, 1 = this fog (y start, z end, zone_fog_color), 2 = no fog.
    zone_fog: vec4<f32>,
    zone_fog_color: vec4<f32>,
};
@group(1) @binding(0) var<uniform> pu: PassU;

@group(2) @binding(0) var t0: texture_2d<f32>;
@group(2) @binding(1) var t1: texture_2d<f32>;
@group(2) @binding(2) var t2: texture_2d<f32>;
@group(2) @binding(3) var t3: texture_2d<f32>;
@group(2) @binding(4) var t4: texture_2d<f32>;
@group(2) @binding(5) var s0: sampler;
@group(2) @binding(6) var s1: sampler;
@group(2) @binding(7) var s2: sampler;
@group(2) @binding(8) var s3: sampler;
@group(2) @binding(9) var s4: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) diffuse: vec4<f32>,
    @location(1) tc0: vec4<f32>,
    @location(2) tc1: vec4<f32>,
    @location(3) tc2: vec4<f32>,
    @location(4) tc3: vec4<f32>,
    @location(5) tc4: vec4<f32>,
    @location(6) depth: f32,
};

// Coordinates of a stage: UV stream, or generated in camera space (D3DTSS_TCI_*), then
// D3DTSS_TEXTURETRANSFORMFLAGS (row vector × matrix).
fn stage_coord(s: StageU, uv0: vec2<f32>, uv1: vec2<f32>, pc: vec3<f32>, nc: vec3<f32>, nl: vec3<f32>) -> vec4<f32> {
    let tci = s.ops.z;
    var c: vec4<f32>;
    let gen = tci & 0xFFFF0000u;
    if (gen == 0x40000u) {
        // Cartoon shader: oT1.x = dp4(n, c5), oT1.y = dp4(n, c6), n normalized (w = 1).
        let n4 = vec4<f32>(nl, 1.0);
        return vec4<f32>(dot(n4, pu.c5), dot(n4, pu.c6), 1.0, 1.0);
    }
    if (gen == 0x10000u) {
        c = vec4<f32>(nc, 1.0);
    } else if (gen == 0x20000u) {
        c = vec4<f32>(pc, 1.0);
    } else if (gen == 0x30000u) {
        // Reflection vector (D3DRS_LOCALVIEWER true): u - 2 n (n·u), u = normalized position.
        let u = normalize(pc);
        c = vec4<f32>(u - 2.0 * nc * dot(nc, u), 1.0);
    } else if ((tci & 0xFFFFu) == 1u) {
        c = vec4<f32>(uv1, 1.0, 1.0);
    } else {
        c = vec4<f32>(uv0, 1.0, 1.0);
    }
    if ((s.ops.w & 0xFFu) != 0u) {
        c = s.m * c;
    }
    return c;
}

@vertex
fn vs_main(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) color: vec4<f32>,
           @location(3) uv0: vec2<f32>, @location(4) uv1: vec2<f32>) -> VOut {
    var o: VOut;
    let bits = pu.flags.w;
    var wp = p;
    if ((bits & 1024u) != 0u) { wp = (pu.l2w * vec4<f32>(p, 1.0)).xyz; }
    var wn = n;
    // Local normal: the one of the mesh's vertex stream, normalized as in the shaders.
    var nl = normalize(n);
    if ((bits & 256u) != 0u) {
        if ((bits & 1024u) != 0u) { nl = normalize(p); }
        else { nl = normalize((pu.w2l * vec4<f32>(p, 1.0)).xyz); }
    }
    if ((bits & (32u | 1024u)) != 0u) {
        wn = (pu.l2w * vec4<f32>(n, 0.0)).xyz;
    }
    if ((bits & 128u) != 0u) {
        // Outline shader: position + normalized normal × c4.w (local frame).
        wp = wp + (pu.l2w * vec4<f32>(normalize(n) * pu.c4.w, 0.0)).xyz;
    }
    // Unreal is left-handed: Y flipped for wgpu's right-handed projection.
    o.pos = fr.view_proj * vec4<f32>(wp.x, -wp.y, wp.z, 1.0);
    if ((bits & 512u) != 0u) {
        // FCanvasUtil: orthographic projection of the screen's pixels (z = 0).
        o.pos = vec4<f32>(p.x, p.y, 0.0, 1.0);
    }
    let pc = (fr.view * vec4<f32>(wp, 1.0)).xyz;
    // Stream without normals (BSP, 32-byte FBspVertex): D3D takes (0, 0, 0), the reflection
    // vector becomes the view direction (DM_LostTemple's window).
    let ncv = (fr.view * vec4<f32>(wn, 0.0)).xyz;
    let nc = select(vec3<f32>(0.0), normalize(ncv), dot(ncv, ncv) > 0.0);
    var d: vec4<f32>;
    if ((bits & 64u) != 0u) {
        // oD0 = c4 (saturated by the hardware).
        d = clamp(pu.c4, vec4<f32>(0.0), vec4<f32>(1.0));
    } else if ((bits & 1u) != 0u) {
        // D3DRS_LIGHTING: emissive = vertex color (D3DRS_EMISSIVEMATERIALSOURCE = COLOR1 if
        // COLORVERTEX, set by SetRes), ambient = D3DRS_AMBIENT × (1, 1, 1) of the D3D material,
        // alpha = the material's diffuse (0.5, 0.5, 0.5, 1).
        var em = vec3<f32>(0.0);
        if ((bits & 2u) != 0u && (bits & 4u) != 0u) {
            em = color.rgb;
        }
        // D3D lights (SetLight): diffuse × material (0.5) × max(N·L, 0) × attenuation
        // 1 / (A1 · d) within the range × cone factor (Theta 0, Falloff 1).
        var lit = vec3<f32>(0.0);
        let nw = normalize(wn);
        for (var i = 0u; i < pu.light_count.x; i = i + 1u) {
            let lp = pu.lights[i * 4u];
            let ld = pu.lights[i * 4u + 1u];
            let lc = pu.lights[i * 4u + 2u];
            let la = pu.lights[i * 4u + 3u];
            var l: vec3<f32>;
            var att = 1.0;
            if (lp.w == 3.0) {
                l = -normalize(ld.xyz);
            } else {
                let v = lp.xyz - wp;
                let dist = length(v);
                if (dist > ld.w) {
                    continue;
                }
                l = v / max(dist, 1e-6);
                att = 1.0 / max(la.x * dist, 1e-6);
                if (lp.w == 2.0) {
                    let rho = dot(normalize(ld.xyz), -l);
                    if (rho <= la.y) {
                        continue;
                    }
                    att = att * clamp((rho - la.y) / (1.0 - la.y), 0.0, 1.0);
                }
            }
            lit = lit + lc.rgb * 0.5 * max(dot(nw, l), 0.0) * att;
        }
        d = vec4<f32>(clamp(em + pu.ambient.rgb + lit, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
    } else if ((bits & 4u) != 0u) {
        d = color;
    } else {
        d = vec4<f32>(1.0);
    }
    o.diffuse = d;
    if (STAGE_COUNT > 0u) { o.tc0 = stage_coord(pu.st[0], uv0, uv1, pc, nc, nl); }
    if (STAGE_COUNT > 1u) { o.tc1 = stage_coord(pu.st[1], uv0, uv1, pc, nc, nl); }
    if (STAGE_COUNT > 2u) { o.tc2 = stage_coord(pu.st[2], uv0, uv1, pc, nc, nl); }
    if (STAGE_COUNT > 3u) { o.tc3 = stage_coord(pu.st[3], uv0, uv1, pc, nc, nl); }
    if (STAGE_COUNT > 4u) { o.tc4 = stage_coord(pu.st[4], uv0, uv1, pc, nc, nl); }
    o.depth = pc.z;
    return o;
}

fn project(s: StageU, c: vec4<f32>) -> vec2<f32> {
    return project_w(s.ops.w, c);
}

fn project_w(f: u32, c: vec4<f32>) -> vec2<f32> {
    if ((f & 0x100u) != 0u) {
        let count = f & 0xFFu;
        var w = c.w;
        if (count == 2u) { w = c.y; } else if (count == 3u) { w = c.z; }
        return c.xy / w;
    }
    return c.xy;
}

fn arg(a: u32, diffuse: vec4<f32>, current: vec4<f32>, tex: vec4<f32>, tf: vec4<f32>) -> vec4<f32> {
    var v: vec4<f32>;
    switch (a & 0xFu) {
        case 0u: { v = diffuse; }
        case 1u: { v = current; }
        case 2u: { v = tex; }
        case 3u: { v = tf; }
        default: { v = vec4<f32>(0.0); }
    }
    if ((a & 0x10u) != 0u) { v = vec4<f32>(1.0) - v; }
    if ((a & 0x20u) != 0u) { v = vec4<f32>(v.a); }
    return v;
}

// One D3DTOP operation on vectors (color) or on their alpha component.
fn top(op: u32, a1: vec4<f32>, a2: vec4<f32>, a0: vec4<f32>, diffuse: vec4<f32>, tex: vec4<f32>,
       current: vec4<f32>, tf: vec4<f32>) -> vec4<f32> {
    switch (op) {
        case 2u: { return a1; }
        case 3u: { return a2; }
        case 4u: { return a1 * a2; }
        case 5u: { return a1 * a2 * 2.0; }
        case 6u: { return a1 * a2 * 4.0; }
        case 7u: { return a1 + a2; }
        case 8u: { return a1 + a2 - vec4<f32>(0.5); }
        case 9u: { return (a1 + a2 - vec4<f32>(0.5)) * 2.0; }
        case 10u: { return a1 - a2; }
        case 11u: { return a1 + a2 - a1 * a2; }
        case 12u: { return mix(a2, a1, diffuse.a); }
        case 13u: { return mix(a2, a1, tex.a); }
        case 14u: { return mix(a2, a1, tf.a); }
        case 15u: { return a1 + a2 * (1.0 - tex.a); }
        case 16u: { return mix(a2, a1, current.a); }
        case 18u: { return vec4<f32>(a1.rgb + a1.a * a2.rgb, a1.a); }
        case 19u: { return vec4<f32>(a1.rgb * a2.rgb + vec3<f32>(a1.a), a1.a); }
        case 20u: { return vec4<f32>((1.0 - a1.a) * a2.rgb + a1.rgb, a1.a); }
        case 21u: { return vec4<f32>((vec3<f32>(1.0) - a1.rgb) * a2.rgb + vec3<f32>(a1.a), a1.a); }
        case 24u: { return vec4<f32>(clamp(4.0 * dot(a1.rgb - vec3<f32>(0.5), a2.rgb - vec3<f32>(0.5)), 0.0, 1.0)); }
        case 25u: { return a0 + a1 * a2; }
        case 26u: { return a0 * a1 + (vec4<f32>(1.0) - a0) * a2; }
        default: { return current; }
    }
}

// One texture stage (body of the former loop): ops = (color, alpha), ca / aa = arguments 0, 1, 2.
fn apply_stage(k: u32, done_flag: ptr<function, bool>, cur: ptr<function, vec4<f32>>, d: vec4<f32>, t: vec4<f32>, tf: vec4<f32>,
         ops: vec2<u32>, ca: vec3<u32>, aa: vec3<u32>) {
    if (*done_flag) { return; }
    if (k >= pu.flags.x || ops.x == 1u) { *done_flag = true; return; }
    let cr = *cur;
    let c = top(ops.x, arg(ca.y, d, cr, t, tf), arg(ca.z, d, cr, t, tf),
                arg(ca.x, d, cr, t, tf), d, t, cr, tf);
    var a = cr.a;
    if (ops.y != 1u) {
        a = top(ops.y, arg(aa.y, d, cr, t, tf), arg(aa.z, d, cr, t, tf),
                arg(aa.x, d, cr, t, tf), d, t, cr, tf).a;
    }
    *cur = clamp(vec4<f32>(c.rgb, a), vec4<f32>(0.0), vec4<f32>(1.0));
}

// Catmull-Rom bicubic read of t0 (9 bilinear reads) at the mip level just larger than the
// screen size: the same large picture stays sharp at any size.
fn sample_fine(uv: vec2<f32>) -> vec4<f32> {
    let dims0 = vec2<f32>(textureDimensions(t0, 0));
    let dx = dpdx(uv * dims0);
    let dy = dpdy(uv * dims0);
    let rho = max(length(dx), length(dy));
    let last = f32(textureNumLevels(t0) - 1u);
    let lvl = clamp(floor(log2(max(rho, 1.0))), 0.0, last);
    let dims = vec2<f32>(textureDimensions(t0, u32(lvl)));
    let p = uv * dims - 0.5;
    let base = floor(p);
    let f = p - base;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let c0 = (base - 0.5) / dims;
    let c3 = (base + 2.5) / dims;
    let c12 = (base + 0.5 + w2 / w12) / dims;
    var r = textureSampleLevel(t0, s0, vec2<f32>(c0.x, c0.y), lvl) * w0.x * w0.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c12.x, c0.y), lvl) * w12.x * w0.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c3.x, c0.y), lvl) * w3.x * w0.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c0.x, c12.y), lvl) * w0.x * w12.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c12.x, c12.y), lvl) * w12.x * w12.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c3.x, c12.y), lvl) * w3.x * w12.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c0.x, c3.y), lvl) * w0.x * w3.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c12.x, c3.y), lvl) * w12.x * w3.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c3.x, c3.y), lvl) * w3.x * w3.y;
    return clamp(r, vec4<f32>(0.0), vec4<f32>(1.0));
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    var tex: array<vec4<f32>, 5>;
    // The number of stages is uniform over the whole draw, including for the LOD computation.
    if (STAGE_COUNT > 0u) {
        let uv0 = project_w(select(pu.st[0].ops.w, OPS0_W, SPECIAL), i.tc0);
        // light_count.y: stage 0 is a picture made from one large master (improved menus):
        // bicubic read from the mip level just larger than the screen size.
        if (pu.light_count.y != 0u) { tex[0] = sample_fine(uv0); } else { tex[0] = textureSample(t0, s0, uv0); }
    }
    if (STAGE_COUNT > 1u) { tex[1] = textureSample(t1, s1, project_w(select(pu.st[1].ops.w, OPS1_W, SPECIAL), i.tc1)); }
    if (STAGE_COUNT > 2u) { tex[2] = textureSample(t2, s2, project_w(select(pu.st[2].ops.w, OPS2_W, SPECIAL), i.tc2)); }
    if (STAGE_COUNT > 3u) { tex[3] = textureSample(t3, s3, project_w(select(pu.st[3].ops.w, OPS3_W, SPECIAL), i.tc3)); }
    if (STAGE_COUNT > 4u) { tex[4] = textureSample(t4, s4, project_w(select(pu.st[4].ops.w, OPS4_W, SPECIAL), i.tc4)); }
    let d = i.diffuse;
    let tf = pu.tfactor;
    var cur = d;
    // The stages one after the other (the earlier loop, unrolled: its operations become
    // constants when the pipeline is specialized); "done" replaces its break.
    var done_flag = false;
    if (STAGE_COUNT > 0u) {
        let s = pu.st[0];
        apply_stage(0u, &done_flag, &cur, d, tex[0], tf, select(s.ops.xy, vec2<u32>(OPS0_C, OPS0_A), SPECIAL), select(s.cargs.xyz, vec3<u32>(OPS0_C0, OPS0_C1, OPS0_C2), SPECIAL), select(s.aargs.xyz, vec3<u32>(OPS0_A0, OPS0_A1, OPS0_A2), SPECIAL));
    }
    if (STAGE_COUNT > 1u) {
        let s = pu.st[1];
        apply_stage(1u, &done_flag, &cur, d, tex[1], tf, select(s.ops.xy, vec2<u32>(OPS1_C, OPS1_A), SPECIAL), select(s.cargs.xyz, vec3<u32>(OPS1_C0, OPS1_C1, OPS1_C2), SPECIAL), select(s.aargs.xyz, vec3<u32>(OPS1_A0, OPS1_A1, OPS1_A2), SPECIAL));
    }
    if (STAGE_COUNT > 2u) {
        let s = pu.st[2];
        apply_stage(2u, &done_flag, &cur, d, tex[2], tf, select(s.ops.xy, vec2<u32>(OPS2_C, OPS2_A), SPECIAL), select(s.cargs.xyz, vec3<u32>(OPS2_C0, OPS2_C1, OPS2_C2), SPECIAL), select(s.aargs.xyz, vec3<u32>(OPS2_A0, OPS2_A1, OPS2_A2), SPECIAL));
    }
    if (STAGE_COUNT > 3u) {
        let s = pu.st[3];
        apply_stage(3u, &done_flag, &cur, d, tex[3], tf, select(s.ops.xy, vec2<u32>(OPS3_C, OPS3_A), SPECIAL), select(s.cargs.xyz, vec3<u32>(OPS3_C0, OPS3_C1, OPS3_C2), SPECIAL), select(s.aargs.xyz, vec3<u32>(OPS3_A0, OPS3_A1, OPS3_A2), SPECIAL));
    }
    if (STAGE_COUNT > 4u) {
        let s = pu.st[4];
        apply_stage(4u, &done_flag, &cur, d, tex[4], tf, select(s.ops.xy, vec2<u32>(OPS4_C, OPS4_A), SPECIAL), select(s.cargs.xyz, vec3<u32>(OPS4_C0, OPS4_C1, OPS4_C2), SPECIAL), select(s.aargs.xyz, vec3<u32>(OPS4_A0, OPS4_A1, OPS4_A2), SPECIAL));
    }
    // D3DRS_ALPHATESTENABLE, D3DRS_ALPHAFUNC = D3DCMP_GREATER, D3DRS_ALPHAREF.
    if (A2C && pu.flags.y != 0u) {
        // Same threshold as below (alpha rounded to 8 bits above ALPHAREF), widened to one pixel.
        let a = (cur.a * 255.0 - (f32(pu.flags.z) + 0.5)) / max(fwidth(cur.a * 255.0), 1.0e-3) + 0.5;
        if (a <= 0.0) {
            discard;
        }
        cur.a = clamp(a, 0.0, 1.0);
    } else if (pu.flags.y != 0u && !(floor(cur.a * 255.0 + 0.5) > f32(pu.flags.z))) {
        discard;
    }
    // Part of a first-person model kept behind the usual near plane while the rest of the model
    // comes closer to the eye (light_count.w: that distance's bits, 0 = none).
    if (pu.light_count.w != 0u && i.depth < bitcast<f32>(pu.light_count.w)) {
        discard;
    }
    let bits = pu.flags.w;
    // Fog of the drawn element's zone when given, otherwise the frame's.
    var fog = fr.fog;
    var fc = fr.fog_color.rgb;
    if (pu.zone_fog.x == 1.0) { fog = vec4<f32>(1.0, pu.zone_fog.y, pu.zone_fog.z, 0.0); fc = pu.zone_fog_color.rgb; }
    else if (pu.zone_fog.x == 2.0) { fog.x = 0.0; }
    if (fog.x != 0.0 && (bits & 8u) == 0u) {
        if ((bits & 16u) != 0u) { fc = pu.fog_color.rgb; }
        let f = clamp((fog.z - i.depth) / (fog.z - fog.y), 0.0, 1.0);
        cur = vec4<f32>(mix(fc, cur.rgb, f), cur.a);
    }
    return cur;
}
"#;

// Separate module: do not change the HLSL or the bytecode of the other draws.
/// Line of SHADER that pushes a vertex out for the outline (shader).
const OUTLINE_PUSH: &str = "        wp = wp + (pu.l2w * vec4<f32>(normalize(n) * pu.c4.w, 0.0)).xyz;";

/// SHADER as the renderer compiles it. Mods build only: a vertex whose normal is almost zero is
/// not pushed out for the outline. Such normals only come from bones shrunk to almost nothing
/// (FirstPersonHideBones hides the parent's gun that way); normalized, they would still be
/// pushed by the full thickness, and the hidden part would show as a black dot.
pub fn main_shader() -> std::borrow::Cow<'static, str> {
    #[cfg(feature = "mods")]
    {
        return SHADER.replacen(OUTLINE_PUSH, "        if (dot(n, n) > 1.0e-4) { wp = wp + (pu.l2w * vec4<f32>(normalize(n) * pu.c4.w, 0.0)).xyz; }", 1).into();
    }
    #[allow(unreachable_code)]
    SHADER.into()
}

pub const IMPACT_SHADER: &str = r#"
override STAGE_COUNT: u32 = 5u;
// Specialized combiner (like the small program the Direct3D 8 driver prepares for each setting
// of the stages): SPECIAL true, the operations and arguments of each stage are constants of the
// pipeline (same values as in pu.st, same computation); false, they are read from the uniforms.
override SPECIAL: bool = false;
// Alpha to coverage (multisampled opaque pass with alpha test): the alpha written becomes the
// pixel's coverage around the reference, instead of the hard cut.
override A2C: bool = false;
override OPS0_C: u32 = 0u;
override OPS0_A: u32 = 0u;
override OPS0_W: u32 = 0u;
override OPS0_C0: u32 = 0u;
override OPS0_C1: u32 = 0u;
override OPS0_C2: u32 = 0u;
override OPS0_A0: u32 = 0u;
override OPS0_A1: u32 = 0u;
override OPS0_A2: u32 = 0u;
override OPS1_C: u32 = 0u;
override OPS1_A: u32 = 0u;
override OPS1_W: u32 = 0u;
override OPS1_C0: u32 = 0u;
override OPS1_C1: u32 = 0u;
override OPS1_C2: u32 = 0u;
override OPS1_A0: u32 = 0u;
override OPS1_A1: u32 = 0u;
override OPS1_A2: u32 = 0u;
override OPS2_C: u32 = 0u;
override OPS2_A: u32 = 0u;
override OPS2_W: u32 = 0u;
override OPS2_C0: u32 = 0u;
override OPS2_C1: u32 = 0u;
override OPS2_C2: u32 = 0u;
override OPS2_A0: u32 = 0u;
override OPS2_A1: u32 = 0u;
override OPS2_A2: u32 = 0u;
override OPS3_C: u32 = 0u;
override OPS3_A: u32 = 0u;
override OPS3_W: u32 = 0u;
override OPS3_C0: u32 = 0u;
override OPS3_C1: u32 = 0u;
override OPS3_C2: u32 = 0u;
override OPS3_A0: u32 = 0u;
override OPS3_A1: u32 = 0u;
override OPS3_A2: u32 = 0u;
override OPS4_C: u32 = 0u;
override OPS4_A: u32 = 0u;
override OPS4_W: u32 = 0u;
override OPS4_C0: u32 = 0u;
override OPS4_C1: u32 = 0u;
override OPS4_C2: u32 = 0u;
override OPS4_A0: u32 = 0u;
override OPS4_A1: u32 = 0u;
override OPS4_A2: u32 = 0u;
struct Frame {
    view_proj: mat4x4<f32>,
    // Direct3D view matrix (world -> camera), by rows: v' = M * v with this layout.
    view: mat4x4<f32>,
    // x: fog on, y: start, z: end.
    fog: vec4<f32>,
    fog_color: vec4<f32>,
    // D3DRS_AMBIENT.
    ambient: vec4<f32>,
};
@group(0) @binding(0) var<uniform> fr: Frame;

struct StageU {
    // color_op, alpha_op, tex_coord_index, transform_flags (0 without a matrix)
    ops: vec4<u32>,
    // color_arg0, color_arg1, color_arg2, result_arg
    cargs: vec4<u32>,
    // alpha_arg0, alpha_arg1, alpha_arg2, 0
    aargs: vec4<u32>,
    m: mat4x4<f32>,
};
struct PassU {
    st: array<StageU, 5>,
    tfactor: vec4<f32>,
    // x: stages used, y: alpha test, z: alpha reference, w: bits (1: lighting,
    // 2: colorvertex, 4: color in the stream, 8: fog ignored, 16: own fog
    // color, 32: local normals, 64: forced diffuse (c4), 128: outline (vertices pushed out),
    // 256: normal = local position, 512: position already in screen coordinates (Canvas))
    flags: vec4<u32>,
    fog_color: vec4<f32>,
    // D3DRS_AMBIENT.
    ambient: vec4<f32>,
    // Constants of D3DDrv's cartoon shaders: c4 (color, outline width in w), c5, c6.
    c4: vec4<f32>,
    c5: vec4<f32>,
    c6: vec4<f32>,
    // The actor's LocalToWorld (rows) and its inverse.
    l2w: mat4x4<f32>,
    w2l: mat4x4<f32>,
    // D3DLIGHT8: position and type (1 point, 2 spot, 3 directional), direction and
    // range, diffuse, (Attenuation1, cos(Phi / 2), 0, 0).
    lights: array<vec4<f32>, 32>,
    light_count: vec4<u32>,
    // Fog of the zone of the drawn element (SetDistanceFog per zone): x 0 = the
    // frame's fog, 1 = this fog (y start, z end, zone_fog_color), 2 = no fog.
    zone_fog: vec4<f32>,
    zone_fog_color: vec4<f32>,
};
@group(1) @binding(0) var<uniform> pu: PassU;

@group(2) @binding(0) var t0: texture_2d<f32>;
@group(2) @binding(1) var t1: texture_2d<f32>;
@group(2) @binding(2) var t2: texture_2d<f32>;
@group(2) @binding(3) var t3: texture_2d<f32>;
@group(2) @binding(4) var t4: texture_2d<f32>;
@group(2) @binding(5) var s0: sampler;
@group(2) @binding(6) var s1: sampler;
@group(2) @binding(7) var s2: sampler;
@group(2) @binding(8) var s3: sampler;
@group(2) @binding(9) var s4: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) diffuse: vec4<f32>,
    @location(1) tc0: vec4<f32>,
    @location(2) tc1: vec4<f32>,
    @location(3) tc2: vec4<f32>,
    @location(4) tc3: vec4<f32>,
    @location(5) tc4: vec4<f32>,
    @location(6) depth: f32,
};

// Coordinates of a stage: UV stream, or generated in camera space (D3DTSS_TCI_*), then
// D3DTSS_TEXTURETRANSFORMFLAGS (row vector × matrix).
fn stage_coord(s: StageU, impact: mat4x4<f32>, index: u32, uv0: vec2<f32>, uv1: vec2<f32>, pc: vec3<f32>, nc: vec3<f32>, nl: vec3<f32>) -> vec4<f32> {
    let tci = s.ops.z;
    var c: vec4<f32>;
    let gen = tci & 0xFFFF0000u;
    if (gen == 0x40000u) {
        // Cartoon shader: oT1.x = dp4(n, c5), oT1.y = dp4(n, c6), n normalized (w = 1).
        let n4 = vec4<f32>(nl, 1.0);
        return vec4<f32>(dot(n4, pu.c5), dot(n4, pu.c6), 1.0, 1.0);
    }
    if (gen == 0x10000u) {
        c = vec4<f32>(nc, 1.0);
    } else if (gen == 0x20000u) {
        c = vec4<f32>(pc, 1.0);
    } else if (gen == 0x30000u) {
        // Reflection vector (D3DRS_LOCALVIEWER true): u - 2 n (n·u), u = normalized position.
        let u = normalize(pc);
        c = vec4<f32>(u - 2.0 * nc * dot(nc, u), 1.0);
    } else if ((tci & 0xFFFFu) == 1u) {
        c = vec4<f32>(uv1, 1.0, 1.0);
    } else {
        c = vec4<f32>(uv0, 1.0, 1.0);
    }
    if ((s.ops.w & 0xFFu) != 0u) {
        var m = s.m;
        if ((pu.flags.w & (2097152u << index)) != 0u) {
            // Each constant keeps its original slot in PassU: two
            // zeros equal at run time do not become the same operand for FXC.
            m = mat4x4<f32>(vec4<f32>(s.m[0].x, impact[0].w, s.m[0].z, s.m[0].w),
                            vec4<f32>(s.m[1].x, impact[1].w, s.m[1].z, s.m[1].w),
                            vec4<f32>(s.m[2].x, impact[2].w, s.m[2].z, s.m[2].w),
                            vec4<f32>(s.m[3].x, impact[3].w, s.m[3].z, s.m[3].w));
        } else if ((pu.flags.w & (65536u << index)) != 0u) {
            if ((pu.flags.w & 67108864u) != 0u) {
                // The selection is a copy, not a floating-point computation. Its predicate
                // uniform is true; it prevents factoring z and w into one product.
                let copy_z = pu.l2w[0].x != pu.l2w[0].y;
                m = mat4x4<f32>(vec4<f32>(impact[0].xyz, select(s.m[0].w, impact[0].z, copy_z)),
                                vec4<f32>(impact[1].xyz, select(s.m[1].w, impact[1].z, copy_z)),
                                vec4<f32>(impact[2].xyz, select(s.m[2].w, impact[2].z, copy_z)),
                                vec4<f32>(impact[3].xyz, select(s.m[3].w, impact[3].z, copy_z)));
            } else if ((pu.flags.w & 134217728u) != 0u) {
                m = mat4x4<f32>(vec4<f32>(impact[0].xyz, s.m[0].w), vec4<f32>(impact[1].xyz, s.m[1].w),
                                vec4<f32>(impact[2].xyz, s.m[2].w), vec4<f32>(impact[3].xyz, s.m[3].w));
            } else { m = impact; }
        }
        // Same chain as the DXBC of the historical path: mul(y), mad(x), mad(z), add(w).
        // Otherwise a matrix product on attributes becomes dp4 under FXC.
        // The uniform condition is true for the impacts accepted by take_impact_matrix.
        // It keeps the Y product rounded before the X mad (FXC swaps them without it).
        var xy = m[1] * c.y;
        if (pu.l2w[0].x != pu.l2w[0].y) {
            xy = fma(m[0], vec4<f32>(c.x), xy);
        }
        c = fma(m[2], vec4<f32>(c.z), xy) + m[3];
    }
    return c;
}

@vertex
fn vs_impacts(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) color: vec4<f32>,
           @location(3) uv0: vec2<f32>, @location(4) uv1: vec2<f32>,
           @location(5) m0: vec4<f32>, @location(6) m1: vec4<f32>,
           @location(7) m2: vec4<f32>, @location(8) m3: vec4<f32>) -> VOut {
    let impact = mat4x4<f32>(m0, m1, m2, m3);
    var o: VOut;
    let bits = pu.flags.w;
    var wp = p;
    if ((bits & 1024u) != 0u) { wp = (pu.l2w * vec4<f32>(p, 1.0)).xyz; }
    var wn = n;
    // Local normal: the one of the mesh's vertex stream, normalized as in the shaders.
    var nl = normalize(n);
    if ((bits & 256u) != 0u) {
        if ((bits & 1024u) != 0u) { nl = normalize(p); }
        else { nl = normalize((pu.w2l * vec4<f32>(p, 1.0)).xyz); }
    }
    if ((bits & (32u | 1024u)) != 0u) {
        wn = (pu.l2w * vec4<f32>(n, 0.0)).xyz;
    }
    if ((bits & 128u) != 0u) {
        // Outline shader: position + normalized normal × c4.w (local frame).
        wp = wp + (pu.l2w * vec4<f32>(normalize(n) * pu.c4.w, 0.0)).xyz;
    }
    // Unreal is left-handed: Y flipped for wgpu's right-handed projection.
    o.pos = fr.view_proj * vec4<f32>(wp.x, -wp.y, wp.z, 1.0);
    if ((bits & 512u) != 0u) {
        // FCanvasUtil: orthographic projection of the screen's pixels (z = 0).
        o.pos = vec4<f32>(p.x, p.y, 0.0, 1.0);
    }
    let pc = (fr.view * vec4<f32>(wp, 1.0)).xyz;
    // Stream without normals (BSP, 32-byte FBspVertex): D3D takes (0, 0, 0), the reflection
    // vector becomes the view direction (DM_LostTemple's window).
    let ncv = (fr.view * vec4<f32>(wn, 0.0)).xyz;
    let nc = select(vec3<f32>(0.0), normalize(ncv), dot(ncv, ncv) > 0.0);
    var d: vec4<f32>;
    if ((bits & 64u) != 0u) {
        // oD0 = c4 (saturated by the hardware).
        d = clamp(pu.c4, vec4<f32>(0.0), vec4<f32>(1.0));
    } else if ((bits & 1u) != 0u) {
        // D3DRS_LIGHTING: emissive = vertex color (D3DRS_EMISSIVEMATERIALSOURCE = COLOR1 if
        // COLORVERTEX, set by SetRes), ambient = D3DRS_AMBIENT × (1, 1, 1) of the D3D material,
        // alpha = the material's diffuse (0.5, 0.5, 0.5, 1).
        var em = vec3<f32>(0.0);
        if ((bits & 2u) != 0u && (bits & 4u) != 0u) {
            em = color.rgb;
        }
        // D3D lights (SetLight): diffuse × material (0.5) × max(N·L, 0) × attenuation
        // 1 / (A1 · d) within the range × cone factor (Theta 0, Falloff 1).
        var lit = vec3<f32>(0.0);
        let nw = normalize(wn);
        for (var i = 0u; i < pu.light_count.x; i = i + 1u) {
            let lp = pu.lights[i * 4u];
            let ld = pu.lights[i * 4u + 1u];
            let lc = pu.lights[i * 4u + 2u];
            let la = pu.lights[i * 4u + 3u];
            var l: vec3<f32>;
            var att = 1.0;
            if (lp.w == 3.0) {
                l = -normalize(ld.xyz);
            } else {
                let v = lp.xyz - wp;
                let dist = length(v);
                if (dist > ld.w) {
                    continue;
                }
                l = v / max(dist, 1e-6);
                att = 1.0 / max(la.x * dist, 1e-6);
                if (lp.w == 2.0) {
                    let rho = dot(normalize(ld.xyz), -l);
                    if (rho <= la.y) {
                        continue;
                    }
                    att = att * clamp((rho - la.y) / (1.0 - la.y), 0.0, 1.0);
                }
            }
            lit = lit + lc.rgb * 0.5 * max(dot(nw, l), 0.0) * att;
        }
        d = vec4<f32>(clamp(em + pu.ambient.rgb + lit, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
    } else if ((bits & 4u) != 0u) {
        d = color;
    } else {
        d = vec4<f32>(1.0);
    }
    o.diffuse = d;
    if (STAGE_COUNT > 0u) { o.tc0 = stage_coord(pu.st[0], impact, 0u, uv0, uv1, pc, nc, nl); }
    if (STAGE_COUNT > 1u) { o.tc1 = stage_coord(pu.st[1], impact, 1u, uv0, uv1, pc, nc, nl); }
    if (STAGE_COUNT > 2u) { o.tc2 = stage_coord(pu.st[2], impact, 2u, uv0, uv1, pc, nc, nl); }
    if (STAGE_COUNT > 3u) { o.tc3 = stage_coord(pu.st[3], impact, 3u, uv0, uv1, pc, nc, nl); }
    if (STAGE_COUNT > 4u) { o.tc4 = stage_coord(pu.st[4], impact, 4u, uv0, uv1, pc, nc, nl); }
    o.depth = pc.z;
    return o;
}

fn project(s: StageU, c: vec4<f32>) -> vec2<f32> {
    return project_w(s.ops.w, c);
}

fn project_w(f: u32, c: vec4<f32>) -> vec2<f32> {
    if ((f & 0x100u) != 0u) {
        let count = f & 0xFFu;
        var w = c.w;
        if (count == 2u) { w = c.y; } else if (count == 3u) { w = c.z; }
        return c.xy / w;
    }
    return c.xy;
}

fn arg(a: u32, diffuse: vec4<f32>, current: vec4<f32>, tex: vec4<f32>, tf: vec4<f32>) -> vec4<f32> {
    var v: vec4<f32>;
    switch (a & 0xFu) {
        case 0u: { v = diffuse; }
        case 1u: { v = current; }
        case 2u: { v = tex; }
        case 3u: { v = tf; }
        default: { v = vec4<f32>(0.0); }
    }
    if ((a & 0x10u) != 0u) { v = vec4<f32>(1.0) - v; }
    if ((a & 0x20u) != 0u) { v = vec4<f32>(v.a); }
    return v;
}

// One D3DTOP operation on vectors (color) or on their alpha component.
fn top(op: u32, a1: vec4<f32>, a2: vec4<f32>, a0: vec4<f32>, diffuse: vec4<f32>, tex: vec4<f32>,
       current: vec4<f32>, tf: vec4<f32>) -> vec4<f32> {
    switch (op) {
        case 2u: { return a1; }
        case 3u: { return a2; }
        case 4u: { return a1 * a2; }
        case 5u: { return a1 * a2 * 2.0; }
        case 6u: { return a1 * a2 * 4.0; }
        case 7u: { return a1 + a2; }
        case 8u: { return a1 + a2 - vec4<f32>(0.5); }
        case 9u: { return (a1 + a2 - vec4<f32>(0.5)) * 2.0; }
        case 10u: { return a1 - a2; }
        case 11u: { return a1 + a2 - a1 * a2; }
        case 12u: { return mix(a2, a1, diffuse.a); }
        case 13u: { return mix(a2, a1, tex.a); }
        case 14u: { return mix(a2, a1, tf.a); }
        case 15u: { return a1 + a2 * (1.0 - tex.a); }
        case 16u: { return mix(a2, a1, current.a); }
        case 18u: { return vec4<f32>(a1.rgb + a1.a * a2.rgb, a1.a); }
        case 19u: { return vec4<f32>(a1.rgb * a2.rgb + vec3<f32>(a1.a), a1.a); }
        case 20u: { return vec4<f32>((1.0 - a1.a) * a2.rgb + a1.rgb, a1.a); }
        case 21u: { return vec4<f32>((vec3<f32>(1.0) - a1.rgb) * a2.rgb + vec3<f32>(a1.a), a1.a); }
        case 24u: { return vec4<f32>(clamp(4.0 * dot(a1.rgb - vec3<f32>(0.5), a2.rgb - vec3<f32>(0.5)), 0.0, 1.0)); }
        case 25u: { return a0 + a1 * a2; }
        case 26u: { return a0 * a1 + (vec4<f32>(1.0) - a0) * a2; }
        default: { return current; }
    }
}

// One texture stage (body of the former loop): ops = (color, alpha), ca / aa = arguments 0, 1, 2.
fn apply_stage(k: u32, done_flag: ptr<function, bool>, cur: ptr<function, vec4<f32>>, d: vec4<f32>, t: vec4<f32>, tf: vec4<f32>,
         ops: vec2<u32>, ca: vec3<u32>, aa: vec3<u32>) {
    if (*done_flag) { return; }
    if (k >= pu.flags.x || ops.x == 1u) { *done_flag = true; return; }
    let cr = *cur;
    let c = top(ops.x, arg(ca.y, d, cr, t, tf), arg(ca.z, d, cr, t, tf),
                arg(ca.x, d, cr, t, tf), d, t, cr, tf);
    var a = cr.a;
    if (ops.y != 1u) {
        a = top(ops.y, arg(aa.y, d, cr, t, tf), arg(aa.z, d, cr, t, tf),
                arg(aa.x, d, cr, t, tf), d, t, cr, tf).a;
    }
    *cur = clamp(vec4<f32>(c.rgb, a), vec4<f32>(0.0), vec4<f32>(1.0));
}

// Catmull-Rom bicubic read of t0 (9 bilinear reads) at the mip level just larger than the
// screen size: the same large picture stays sharp at any size.
fn sample_fine(uv: vec2<f32>) -> vec4<f32> {
    let dims0 = vec2<f32>(textureDimensions(t0, 0));
    let dx = dpdx(uv * dims0);
    let dy = dpdy(uv * dims0);
    let rho = max(length(dx), length(dy));
    let last = f32(textureNumLevels(t0) - 1u);
    let lvl = clamp(floor(log2(max(rho, 1.0))), 0.0, last);
    let dims = vec2<f32>(textureDimensions(t0, u32(lvl)));
    let p = uv * dims - 0.5;
    let base = floor(p);
    let f = p - base;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let c0 = (base - 0.5) / dims;
    let c3 = (base + 2.5) / dims;
    let c12 = (base + 0.5 + w2 / w12) / dims;
    var r = textureSampleLevel(t0, s0, vec2<f32>(c0.x, c0.y), lvl) * w0.x * w0.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c12.x, c0.y), lvl) * w12.x * w0.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c3.x, c0.y), lvl) * w3.x * w0.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c0.x, c12.y), lvl) * w0.x * w12.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c12.x, c12.y), lvl) * w12.x * w12.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c3.x, c12.y), lvl) * w3.x * w12.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c0.x, c3.y), lvl) * w0.x * w3.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c12.x, c3.y), lvl) * w12.x * w3.y;
    r += textureSampleLevel(t0, s0, vec2<f32>(c3.x, c3.y), lvl) * w3.x * w3.y;
    return clamp(r, vec4<f32>(0.0), vec4<f32>(1.0));
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    var tex: array<vec4<f32>, 5>;
    // The number of stages is uniform over the whole draw, including for the LOD computation.
    if (STAGE_COUNT > 0u) {
        let uv0 = project_w(select(pu.st[0].ops.w, OPS0_W, SPECIAL), i.tc0);
        // light_count.y: stage 0 is a picture made from one large master (improved menus):
        // bicubic read from the mip level just larger than the screen size.
        if (pu.light_count.y != 0u) { tex[0] = sample_fine(uv0); } else { tex[0] = textureSample(t0, s0, uv0); }
    }
    if (STAGE_COUNT > 1u) { tex[1] = textureSample(t1, s1, project_w(select(pu.st[1].ops.w, OPS1_W, SPECIAL), i.tc1)); }
    if (STAGE_COUNT > 2u) { tex[2] = textureSample(t2, s2, project_w(select(pu.st[2].ops.w, OPS2_W, SPECIAL), i.tc2)); }
    if (STAGE_COUNT > 3u) { tex[3] = textureSample(t3, s3, project_w(select(pu.st[3].ops.w, OPS3_W, SPECIAL), i.tc3)); }
    if (STAGE_COUNT > 4u) { tex[4] = textureSample(t4, s4, project_w(select(pu.st[4].ops.w, OPS4_W, SPECIAL), i.tc4)); }
    let d = i.diffuse;
    let tf = pu.tfactor;
    var cur = d;
    // The stages one after the other (the earlier loop, unrolled: its operations become
    // constants when the pipeline is specialized); "done" replaces its break.
    var done_flag = false;
    if (STAGE_COUNT > 0u) {
        let s = pu.st[0];
        apply_stage(0u, &done_flag, &cur, d, tex[0], tf, select(s.ops.xy, vec2<u32>(OPS0_C, OPS0_A), SPECIAL), select(s.cargs.xyz, vec3<u32>(OPS0_C0, OPS0_C1, OPS0_C2), SPECIAL), select(s.aargs.xyz, vec3<u32>(OPS0_A0, OPS0_A1, OPS0_A2), SPECIAL));
    }
    if (STAGE_COUNT > 1u) {
        let s = pu.st[1];
        apply_stage(1u, &done_flag, &cur, d, tex[1], tf, select(s.ops.xy, vec2<u32>(OPS1_C, OPS1_A), SPECIAL), select(s.cargs.xyz, vec3<u32>(OPS1_C0, OPS1_C1, OPS1_C2), SPECIAL), select(s.aargs.xyz, vec3<u32>(OPS1_A0, OPS1_A1, OPS1_A2), SPECIAL));
    }
    if (STAGE_COUNT > 2u) {
        let s = pu.st[2];
        apply_stage(2u, &done_flag, &cur, d, tex[2], tf, select(s.ops.xy, vec2<u32>(OPS2_C, OPS2_A), SPECIAL), select(s.cargs.xyz, vec3<u32>(OPS2_C0, OPS2_C1, OPS2_C2), SPECIAL), select(s.aargs.xyz, vec3<u32>(OPS2_A0, OPS2_A1, OPS2_A2), SPECIAL));
    }
    if (STAGE_COUNT > 3u) {
        let s = pu.st[3];
        apply_stage(3u, &done_flag, &cur, d, tex[3], tf, select(s.ops.xy, vec2<u32>(OPS3_C, OPS3_A), SPECIAL), select(s.cargs.xyz, vec3<u32>(OPS3_C0, OPS3_C1, OPS3_C2), SPECIAL), select(s.aargs.xyz, vec3<u32>(OPS3_A0, OPS3_A1, OPS3_A2), SPECIAL));
    }
    if (STAGE_COUNT > 4u) {
        let s = pu.st[4];
        apply_stage(4u, &done_flag, &cur, d, tex[4], tf, select(s.ops.xy, vec2<u32>(OPS4_C, OPS4_A), SPECIAL), select(s.cargs.xyz, vec3<u32>(OPS4_C0, OPS4_C1, OPS4_C2), SPECIAL), select(s.aargs.xyz, vec3<u32>(OPS4_A0, OPS4_A1, OPS4_A2), SPECIAL));
    }
    // D3DRS_ALPHATESTENABLE, D3DRS_ALPHAFUNC = D3DCMP_GREATER, D3DRS_ALPHAREF.
    if (A2C && pu.flags.y != 0u) {
        // Same threshold as below (alpha rounded to 8 bits above ALPHAREF), widened to one pixel.
        let a = (cur.a * 255.0 - (f32(pu.flags.z) + 0.5)) / max(fwidth(cur.a * 255.0), 1.0e-3) + 0.5;
        if (a <= 0.0) {
            discard;
        }
        cur.a = clamp(a, 0.0, 1.0);
    } else if (pu.flags.y != 0u && !(floor(cur.a * 255.0 + 0.5) > f32(pu.flags.z))) {
        discard;
    }
    // Part of a first-person model kept behind the usual near plane while the rest of the model
    // comes closer to the eye (light_count.w: that distance's bits, 0 = none).
    if (pu.light_count.w != 0u && i.depth < bitcast<f32>(pu.light_count.w)) {
        discard;
    }
    let bits = pu.flags.w;
    // Fog of the drawn element's zone when given, otherwise the frame's.
    var fog = fr.fog;
    var fc = fr.fog_color.rgb;
    if (pu.zone_fog.x == 1.0) { fog = vec4<f32>(1.0, pu.zone_fog.y, pu.zone_fog.z, 0.0); fc = pu.zone_fog_color.rgb; }
    else if (pu.zone_fog.x == 2.0) { fog.x = 0.0; }
    if (fog.x != 0.0 && (bits & 8u) == 0u) {
        if ((bits & 16u) != 0u) { fc = pu.fog_color.rgb; }
        let f = clamp((fog.z - i.depth) / (fog.z - fog.y), 0.0, 1.0);
        cur = vec4<f32>(mix(fc, cur.rgb, f), cur.a);
    }
    return cur;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FrameUniform {
    pub view_proj: [[f32; 4]; 4],
    pub view: [f32; 16],
    pub fog: [f32; 4],
    pub fog_color: [f32; 4],
    pub ambient: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StageUniform {
    ops: [u32; 4],
    cargs: [u32; 4],
    aargs: [u32; 4],
    m: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PassUniform {
    st: [StageUniform; STAGES],
    tfactor: [f32; 4],
    flags: [u32; 4],
    fog_color: [f32; 4],
    ambient: [f32; 4],
    pub c4: [f32; 4],
    pub c5: [f32; 4],
    pub c6: [f32; 4],
    pub l2w: [f32; 16],
    pub w2l: [f32; 16],
    /// D3DLIGHT8 of the eight lights of FD3DRenderInterface::SetLight, four vectors
    /// each: position and type, direction and range, diffuse, (Attenuation1, cos(Phi / 2), 0, 0).
    pub lights: [[f32; 4]; 32],
    /// x: lights enabled; w: near cut (`set_near_cut`).
    pub light_count: [u32; 4],
    /// Fog of the drawn element's zone (DrawBits::zone_fog): x 0 frame fog, 1 this one, 2 none.
    pub zone_fog: [f32; 4],
    pub zone_fog_color: [f32; 4],
}

/// Size reserved per pass in the uniform buffer (dynamic offset alignment).
pub const PASS_STRIDE: u64 = 2048;

#[cfg(test)]
#[test]
fn matrices_impacts_two_stages_bytes_and_fallback() {
    for affine in [false, true] {
        for gradient_first in [false, true] {
            let mut u = PassUniform::new(&Pass::default(), DrawBits::default());
            u.flags[0] = 2;
            let main: [f32; 16] = std::array::from_fn(|i| {
                if i % 4 == 3 { if affine { if i == 15 { 1. } else { 0. } } else { (i - 1) as f32 } }
                else { i as f32 }
            });
            let mut grad = [0.; 16]; grad[15] = 1.;
            for i in 0..4 { grad[i * 4 + 1] = -(i as f32); }
            for (i, m) in if gradient_first { [grad, main] } else { [main, grad] }.into_iter().enumerate() {
                u.st[i].m = m; u.st[i].ops[2] = 0x20000; u.st[i].ops[3] = 3;
            }
            let original = u;
            let packed = u.take_impact_matrix().expect("exact shape recognised");
            assert!(u.impact());
            u.restore_impact_matrix(packed);
            assert_eq!(bytemuck::bytes_of(&u), bytemuck::bytes_of(&original));
            let mut incompatible = original;
            incompatible.st[if gradient_first { 0 } else { 1 }].m[0] = -0.;
            let before = incompatible;
            assert!(incompatible.take_impact_matrix().is_none());
            assert_eq!(bytemuck::bytes_of(&incompatible), bytemuck::bytes_of(&before));
        }
    }
}

/// Spotlight Phi: cone × 0.03125 × 1.5707964 (FPU qword constants, with π/2 already rounded to float; a single rounding to 24 bits),
/// at most π.
pub fn spot_phi(cone: u8) -> f32 {
    let phi = (cone as f64 * 0.03125 * 1.570_796_370_506_286_6) as f32;
    if phi <= std::f32::consts::PI {
        phi
    } else {
        std::f32::consts::PI
    }
}

/// FD3DRenderInterface::SetLight: D3DLIGHT8 of an FLight (diffuse = colour,
/// specular and ambient zero); effect 0x14 (sun): directional, effects 8 and 12:
/// spotlight (Phi = cone / 32 × π/2, at most π; Theta 0; Falloff 1), otherwise point;
/// attenuation (0, 2 / radius, 0), range = radius.
pub fn d3d_light(l: &openxiii_assets::lighting::Light) -> [[f32; 4]; 4] {
    let (kind, phi) = match l.effect {
        0x14 => (3.0, 0.0),
        0x0c | 0x08 => (2.0, spot_phi(l.cone)),
        _ => (1.0, 0.0),
    };
    let atten1 = 2.0 / l.radius;
    [
        [l.location[0], l.location[1], l.location[2], kind],
        [l.direction[0], l.direction[1], l.direction[2], l.radius],
        [l.color[0], l.color[1], l.color[2], 1.0],
        [atten1, (phi * 0.5).cos(), 0.0, 0.0],
    ]
}

/// D3DCOLOR 0xAARRGGBB as float RGBA.
pub fn d3dcolor(c: u32) -> [f32; 4] {
    [((c >> 16) & 0xFF) as f32 / 255.0, ((c >> 8) & 0xFF) as f32 / 255.0, (c & 0xFF) as f32 / 255.0, (c >> 24) as f32 / 255.0]
}

/// Engine state of a batch, in addition to the pass (EnableLighting, vertex streams).
#[derive(Clone, Copy, Default, Debug)]
pub struct DrawBits {
    pub lighting: bool,
    pub color_vertex: bool,
    pub has_color: bool,
    pub ambient: [u8; 3],
    pub local_normals: bool,
    /// Diffuse forced by c4 (cartoon shader).
    pub diffuse_c4: bool,
    /// Outline pass.
    pub outline: bool,
    /// Normal = local position (shader).
    pub position_normals: bool,
    /// Vertices in screen coordinates (Canvas primitives).
    pub screen: bool,
    /// Sets the fog of the zone of each drawn element (SetDistanceFog): see zone_fog_bits. Zero: the frame's fog.
    pub zone_fog: [u32; 4],
}

/// DrawBits::zone_fog of a zone: SetDistanceFog(bDistanceFog, DistanceFogStart, DistanceFogEnd,
/// DistanceFogColor) of its ZoneInfo (+0x1FC & 4, +0x214, +0x218, +0x210): [1, start, end, RGB] or
/// [2, 0, 0, 0] without fog.
pub fn zone_fog_bits(fog: Option<(f32, f32, [u8; 3])>) -> [u32; 4] {
    match fog {
        Some((start, end, c)) => [1, start.to_bits(), end.to_bits(), (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32],
        None => [2, 0, 0, 0],
    }
}

/// Coordinate generation code specific to the cartoon shader (T1 = N·c5, N·c6).
pub const TCI_CARTOON: u32 = 0x40000;

impl PassUniform {
    pub fn impact(&self) -> bool { self.flags[3] & (31 << 16) != 0 }

    /// Moves the bytes of the already translated matrix, without any new float computation.
    pub fn take_impact_matrix(&mut self) -> Option<[f32; 16]> {
        if self.l2w[0].to_bits() != 1f32.to_bits() || self.l2w[1].to_bits() != 0 { return None; }
        let n = self.stage_count() as usize;
        let eligible = |s: &StageUniform| s.ops[2] == 0x20000 && s.ops[3] & 255 != 0;
        let first = self.st[..n].iter().find(|s| eligible(s))?.m;
        let second = self.st[..n].iter().find(|s| eligible(s) && s.m.map(f32::to_bits) != first.map(f32::to_bits)).map(|s| s.m);
        let (matrix, gradient, packed) = if let Some(second) = second {
            // Only the observed forms, recognised by their bits (including +0).
            let gradient = |m: &[f32; 16]| (0..16).all(|i| i % 4 == 1 || m[i].to_bits() == if i == 15 { 1f32.to_bits() } else { 0 });
            let (main, grad) = if gradient(&second) { (first, second) } else if gradient(&first) { (second, first) } else { return None };
            let mode = if (0..4).all(|i| main[i * 4 + 2].to_bits() == main[i * 4 + 3].to_bits()) { 1 << 26 }
                else if (0..4).all(|i| main[i * 4 + 3].to_bits() == if i == 3 { 1f32.to_bits() } else { 0 }) { 1 << 27 }
                else { return None };
            if self.st[..n].iter().any(|s| eligible(s) && s.m.map(f32::to_bits) != main.map(f32::to_bits) && s.m.map(f32::to_bits) != grad.map(f32::to_bits)) { return None; }
            let stream = std::array::from_fn(|i| if i % 4 == 3 { grad[i - 2] } else { main[i] });
            self.flags[3] |= mode;
            (main, Some(grad), stream)
        } else { (first, None, first) };
        for (i, s) in self.st[..n].iter_mut().enumerate() {
            if eligible(s) && s.m.map(f32::to_bits) == matrix.map(f32::to_bits) {
                if self.flags[3] & (1 << 27) != 0 {
                    for k in 0..16 { if k % 4 != 3 { s.m[k] = 0.; } }
                } else { s.m = [0.; 16]; }
                self.flags[3] |= 1 << (16 + i);
            } else if eligible(s) && gradient.is_some_and(|g| s.m.map(f32::to_bits) == g.map(f32::to_bits)) {
                for k in 0..4 { s.m[k * 4 + 1] = 0.; }
                self.flags[3] |= 1 << (21 + i);
            }
        }
        Some(packed)
    }

    pub fn restore_impact_matrix(&mut self, matrix: [f32; 16]) {
        let gradient = std::array::from_fn(|i| if i % 4 == 1 { matrix[i + 2] } else if i == 15 { self.l2w[0] } else { self.l2w[1] });
        let mut main = matrix;
        if self.flags[3] & (1 << 26) != 0 {
            for i in 0..4 { main[i * 4 + 3] = matrix[i * 4 + 2]; }
        } else if self.flags[3] & (1 << 27) != 0 {
            for i in 0..4 { main[i * 4 + 3] = if i == 3 { self.l2w[0] } else { self.l2w[1] }; }
        }
        for (i, s) in self.st.iter_mut().enumerate() {
            if self.flags[3] & (1 << (16 + i)) != 0 { s.m = main; }
            else if self.flags[3] & (1 << (21 + i)) != 0 { s.m = gradient; }
        }
        self.flags[3] &= !(((1 << 12) - 1) << 16);
    }

    pub fn stage_count(&self) -> u8 { self.flags[0].min(STAGES as u32) as u8 }

    /// Stage operations and arguments for a specialised pipeline (None if a value does not fit
    /// in 16 bits: the pipeline then reads the uniforms).
    pub fn combine(&self) -> Option<[[u16; 9]; STAGES]> {
        let mut out = [[0u16; 9]; STAGES];
        for (o, s) in out.iter_mut().zip(&self.st) {
            let v = [s.ops[0], s.ops[1], s.ops[3], s.cargs[0], s.cargs[1], s.cargs[2], s.aargs[0], s.aargs[1], s.aargs[2]];
            for (d, x) in o.iter_mut().zip(v) {
                *d = u16::try_from(x).ok()?;
            }
        }
        Some(out)
    }

    /// Refresh provides the draw's matrix, even if the HUD has changed the actor since.
    pub fn set_dyn_refresh(&mut self, refresh: Option<&openxiii_vm::visibility::DynRefresh>) {
        if let Some(refresh)=refresh {
            self.l2w=refresh.matrix;
            self.flags[3]|=1024;
        }
    }

    pub fn new(pass: &Pass, bits: DrawBits) -> Self {
        let stage = |s: &Stage| StageUniform {
            ops: [s.color_op, s.alpha_op, s.tex_coord_index, if s.use_matrix { s.transform_flags } else { 0 }],
            cargs: [s.color_arg0, s.color_arg1, s.color_arg2, s.result_arg],
            aargs: [s.alpha_arg0, s.alpha_arg1, s.alpha_arg2, 0],
            m: s.matrix,
        };
        let flags = (bits.lighting as u32)
            | (bits.color_vertex as u32) << 1
            | (bits.has_color as u32) << 2
            | (pass.ignore_fog as u32) << 3
            | (pass.override_fog_color as u32) << 4
            | (bits.local_normals as u32) << 5
            | (bits.diffuse_c4 as u32) << 6
            | (bits.outline as u32) << 7
            | (bits.position_normals as u32) << 8
            | (bits.screen as u32) << 9;
        PassUniform {
            st: std::array::from_fn(|i| stage(&pass.stages[i])),
            tfactor: d3dcolor(pass.tfactor),
            flags: [pass.stages_used as u32, pass.alpha_test as u32, pass.alpha_ref as u32, flags],
            fog_color: d3dcolor(pass.fog_color),
            ambient: [bits.ambient[0] as f32 / 255.0, bits.ambient[1] as f32 / 255.0, bits.ambient[2] as f32 / 255.0, 1.0],
            c4: [0.0; 4],
            c5: [0.0; 4],
            c6: [0.0; 4],
            l2w: openxiii_assets::material::IDENTITY,
            w2l: openxiii_assets::material::IDENTITY,
            lights: [[0.0; 4]; 32],
            light_count: [0; 4],
            zone_fog: [bits.zone_fog[0] as f32, f32::from_bits(bits.zone_fog[1]), f32::from_bits(bits.zone_fog[2]), 0.0],
            zone_fog_color: d3dcolor(bits.zone_fog[3]),
        }
    }

    /// Fragments closer to the eye than `distance` (camera space) are not drawn.
    pub fn set_near_cut(&mut self, distance: f32) {
        self.light_count[3] = distance.to_bits();
    }

    /// Lights enabled by SetLight (at most eight).
    pub fn set_lights(&mut self, lights: &[[[f32; 4]; 4]]) {
        let n = lights.len().min(8);
        for (i, l) in lights.iter().take(n).enumerate() {
            self.lights[i * 4..i * 4 + 4].copy_from_slice(l);
        }
        self.light_count[0] = n as u32;
    }
}

/// Render states of a pass that require a separate wgpu pipeline.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PipelineKey {
    /// Second per-vertex stream, reserved for compatible impacts.
    pub impact: bool,
    pub stage_count: u8,
    pub blend: Option<(u32, u32)>,
    pub z_test: bool,
    pub z_write: bool,
    /// 0: D3DCULL_CW, 1: D3DCULL_CCW, 2: D3DCULL_NONE.
    pub cull: u8,
    /// D3DPT_LINELIST (FCanvasUtil::DrawLine).
    pub lines: bool,
    /// FD3DRenderInterface::SetZBias (D3DRS_ZBIAS, 0 to 16).
    pub z_bias: u8,
    /// Specialised combiner: per stage, colour and alpha operations, the shader's D3DTSS_TEXTURETRANSFORMFLAGS
    /// (ops.w), colour arguments 0-2 and alpha 0-2 (shader constants OPSk_*); None:
    /// read from the uniforms.
    pub combine: Option<[[u16; 9]; STAGES]>,
    /// Opaque pass with D3DRS_ALPHATESTENABLE: with multisampling, its alpha also gives the
    /// pixel's coverage (alpha to coverage) instead of the hard cut. Same pipeline without it.
    pub alpha_test: bool,
}

/// Names of the specialised combiner's constants, in the order of `PipelineKey::combine`.
pub const COMBINE_NAMES: [[&str; 9]; STAGES] = {
    const fn line_value(k: usize) -> [&'static str; 9] {
        const N: [[&str; 9]; 5] = [
            ["OPS0_C", "OPS0_A", "OPS0_W", "OPS0_C0", "OPS0_C1", "OPS0_C2", "OPS0_A0", "OPS0_A1", "OPS0_A2"],
            ["OPS1_C", "OPS1_A", "OPS1_W", "OPS1_C0", "OPS1_C1", "OPS1_C2", "OPS1_A0", "OPS1_A1", "OPS1_A2"],
            ["OPS2_C", "OPS2_A", "OPS2_W", "OPS2_C0", "OPS2_C1", "OPS2_C2", "OPS2_A0", "OPS2_A1", "OPS2_A2"],
            ["OPS3_C", "OPS3_A", "OPS3_W", "OPS3_C0", "OPS3_C1", "OPS3_C2", "OPS3_A0", "OPS3_A1", "OPS3_A2"],
            ["OPS4_C", "OPS4_A", "OPS4_W", "OPS4_C0", "OPS4_C1", "OPS4_C2", "OPS4_A0", "OPS4_A1", "OPS4_A2"],
        ];
        N[k]
    }
    [line_value(0), line_value(1), line_value(2), line_value(3), line_value(4)]
};

impl PipelineKey {
    /// SetMaterialBlending: ALPHABLENDENABLE, SRCBLEND/DESTBLEND, ZFUNC,
    /// ZWRITEENABLE, CULLMODE (TwoSided: D3DCULL_NONE, otherwise SetCullMode's mode).
    pub fn new(pass: &Pass, cull: u8) -> Self {
        PipelineKey {
            impact: false,
            stage_count: pass.stages_used.min(STAGES) as u8,
            blend: pass.alpha_blending.then_some((pass.src_blend, pass.dest_blend)),
            z_test: pass.z_test,
            z_write: pass.z_write,
            cull: if pass.two_sided { 2 } else { cull },
            lines: false,
            z_bias: 0,
            combine: None,
            alpha_test: pass.alpha_test && !pass.alpha_blending,
        }
    }
}

fn factor(b: u32) -> wgpu::BlendFactor {
    use wgpu::BlendFactor::*;
    match b {
        1 => Zero,
        2 => One,
        3 => Src,
        4 => OneMinusSrc,
        5 => SrcAlpha,
        6 => OneMinusSrcAlpha,
        7 => DstAlpha,
        8 => OneMinusDstAlpha,
        9 => Dst,
        10 => OneMinusDst,
        11 => SrcAlphaSaturated,
        _ => One,
    }
}

pub fn blend_state(key: &PipelineKey) -> Option<wgpu::BlendState> {
    key.blend.map(|(s, d)| {
        let c = wgpu::BlendComponent { src_factor: factor(s), dst_factor: factor(d), operation: wgpu::BlendOperation::Add };
        wgpu::BlendState { color: c, alpha: c }
    })
}

/// D3DCULL_CW removes triangles that are clockwise on screen. Converting to right-handed
/// (Y flipped) reverses the winding: a triangle clockwise for Direct3D is counter-clockwise here.
pub fn cull_face(cull: u8) -> Option<wgpu::Face> {
    match cull {
        0 => Some(wgpu::Face::Back),
        1 => Some(wgpu::Face::Front),
        _ => None,
    }
}

/// D3DTADDRESS addressing mode as a wgpu mode.
pub fn address(a: u32) -> wgpu::AddressMode {
    match a {
        1 => wgpu::AddressMode::Repeat,
        2 => wgpu::AddressMode::MirrorRepeat,
        _ => wgpu::AddressMode::ClampToEdge,
    }
}

#[cfg(test)]
mod kept_impacts {
    #[test]
    fn main_shader_outline_push() {
        assert_eq!(super::SHADER.matches(super::OUTLINE_PUSH).count(), 1);
        let s = super::main_shader();
        #[cfg(feature = "mods")]
        assert!(s.contains("if (dot(n, n) > 1.0e-4)") && !s.contains(super::OUTLINE_PUSH));
        #[cfg(not(feature = "mods"))]
        assert_eq!(s, super::SHADER);
        wgpu::naga::front::wgsl::parse_str(&s).expect("valid WGSL");
    }

    /// IMPACT_SHADER is a copy of SHADER: only stage_coord's impact matrix, its calls
    /// and the vs_impacts entry differ. Any other change to SHADER must be mirrored there.
    #[test]
    fn program_impacts_follows_main_program() {
        let mut i = super::IMPACT_SHADER.to_string();
        let start_value = i.find("        var m = s.m;\n").expect("start of the impacts block");
        let end_motif = "        c = fma(m[2], vec4<f32>(c.z), xy) + m[3];\n";
        let end_value = i.find(end_motif).expect("end of the impacts block") + end_motif.len();
        i.replace_range(start_value..end_value, "        c = s.m * c;\n");
        i = i.replace(
            "fn stage_coord(s: StageU, impact: mat4x4<f32>, index: u32, uv0: vec2<f32>,",
            "fn stage_coord(s: StageU, uv0: vec2<f32>,",
        );
        for k in 0..5 {
            i = i.replace(&format!("stage_coord(pu.st[{k}], impact, {k}u, uv0"), &format!("stage_coord(pu.st[{k}], uv0"));
        }
        i = i.replace(
            "fn vs_impacts(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) color: vec4<f32>,\n           @location(3) uv0: vec2<f32>, @location(4) uv1: vec2<f32>,\n           @location(5) m0: vec4<f32>, @location(6) m1: vec4<f32>,\n           @location(7) m2: vec4<f32>, @location(8) m3: vec4<f32>) -> VOut {\n    let impact = mat4x4<f32>(m0, m1, m2, m3);\n",
            "fn vs_main(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) color: vec4<f32>,\n           @location(3) uv0: vec2<f32>, @location(4) uv1: vec2<f32>) -> VOut {\n",
        );
        let a: Vec<&str> = super::SHADER.lines().collect();
        let b: Vec<&str> = i.lines().collect();
        if let Some(n) = a.iter().zip(&b).position(|(x, y)| x != y) {
            panic!("IMPACT_SHADER differs from SHADER at line {}:\n  SHADER        : {}\n  IMPACT_SHADER : {}", n + 1, a[n], b[n]);
        }
        assert_eq!(a.len(), b.len(), "IMPACT_SHADER and SHADER do not have the same number of lines");
    }
}