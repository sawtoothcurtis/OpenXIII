//! Loading animation (Engine.dll, UEngLoadingAnimationThread, constructor,
//! RenderAnimation): during LoadMap, the UWinLoadingAnimationThreadLauncher thread
//! (Window.dll, RenderAnimation) redraws the screen every 75 + rand() % 50 ms:
//! screen cleared to black, red "TA"s (XIIICine.effets.Ta) and orange impacts
//! (XIIICine.effets.impactpoing2A) scattered in clusters of 14 that shrink and fade, and
//! "Chargement" (Engine's XIIIMessage.Loading) followed by 0 to 3 "." at the bottom right, font
//! XIIIFonts.PoliceF20, colour (R 255, G 192, B 64).
//!
//! appRand/appFrand (Core) call rand() from MSVCR70.dll, the multithreaded library: its
//! state (_tiddata->_holdrand) is per thread and is 1 in a new thread. RenderAnimation
//! and the Sleep run on the animation thread, created once: their sequence starts at 1 and lasts
//! the whole session, without touching that of the game thread.

use crate::value::Value;
use crate::Vm;

/// One drawn cell (24 bytes at +0x78 in the original object): centre position, size,
/// material (0: TA, 1: impact), FColor colour (B, G, R, A).
#[derive(Debug, Clone, Copy)]
struct Sprite {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    material: usize,
    color: [u8; 4],
}

/// Animation state (UEngLoadingAnimationThread +0x70 cluster counter, +0x74 number of
/// cells, +0x9D8.. direction, position and delay), and the dot counter.
#[derive(Debug, Clone)]
pub struct LoadingAnimation {
    count: i32,
    sprites: Vec<Sprite>,
    pos: [f32; 2],
    dir: [f32; 2],
    delay: f32,
    dots: i32,
    seed: u32,
    text: String,
}

const MATERIALS: [&str; 2] = ["#canvas:2:0:127|XIIICine.effets.Ta", "#canvas:2:0:127|XIIICine.effets.impactpoing2A"];
const FONT: &str = "XIIIFonts.PoliceF20";

impl LoadingAnimation {
    /// Constructor: localized text ("Loading" as fallback), counter at 0x18, no cell.
    pub fn new(vm: &mut Vm, seed: u32) -> Self {
        let mut text = vm.localize("XIIIMessage", "Loading", "Engine", true);
        if text.is_empty() {
            text = "Loading".into();
        }
        LoadingAnimation { count: 0x18, sprites: Vec::new(), pos: [0.0; 2], dir: [0.0; 2], delay: 0.0, dots: 0, seed, text }
    }

    fn rand(&mut self) -> i32 {
        self.seed = self.seed.wrapping_mul(214_013).wrapping_add(2_531_011);
        ((self.seed >> 16) & 0x7fff) as i32
    }

    fn frand(&mut self) -> f32 {
        self.rand() as f32 * 3.051_851e-5
    }

    /// Thread wait after a frame (Sleep(rand() % 50 + 75)), in milliseconds.
    pub fn pause_ms(&mut self) -> u64 {
        (self.rand() % 50 + 75) as u64
    }

    /// Cells to draw for the frame (in list order, before they shrink) and bottom text; the cells are then
    /// shrunk by 4, their alpha drops by 0x32 and those that would have only 0x32 or less left are removed
    /// (RenderAnimation), then "Chargement" gets one "." per dot threshold (3, 7, 11).
    pub(crate) fn draw_list(&mut self) -> (Vec<Sprite>, String) {
        let drawn = self.sprites.clone();
        let mut kept = Vec::new();
        for s in &drawn {
            if s.color[3] >= 0x33 {
                let mut n = *s;
                n.w -= 4.0;
                n.h -= 4.0;
                n.color[3] -= 0x32;
                kept.push(n);
            }
        }
        self.sprites = kept;
        let mut text = self.text.clone();
        for limit in [3, 7, 11] {
            if self.dots > limit {
                text.push_str(" .");
            }
        }
        (drawn, text)
    }

    /// One frame: RenderAnimation (viewport size `w` × `h`).
    fn step(&mut self, w: i32, h: i32) {
        if self.count == 0 {
            self.delay = 0.1;
            self.pos[0] = (self.rand() % (w - 300) + 150) as f32;
            self.pos[1] = (self.rand() % (h - 200) + 100) as f32;
            self.dir[0] = self.frand() - 0.5;
            self.dir[1] = self.frand() - 0.5;
            self.count += 1;
        }
        if self.count < 15 {
            let left = self.delay - 0.1;
            self.delay = left;
            if left <= 0.0 {
                self.sprites.push(Sprite { x: self.pos[0], y: self.pos[1], w: 50.0, h: 50.0, material: 0, color: [0, 0, 0xff, 0xff] });
                let x = (self.frand() - 0.5) * 60.0 + self.pos[0];
                let y = (self.frand() - 0.5) * 60.0 + self.pos[1];
                self.sprites.push(Sprite { x, y, w: 50.0, h: 50.0, material: 1, color: [0x40, 0xc0, 0xff, 0xff] });
                // The drawn direction is multiplied by 0: only the random jump counts.
                self.pos[0] += self.dir[0] * 0.0 + (self.frand() - 0.5) * 150.0;
                let dy = (self.frand() - 0.5) * 150.0;
                self.delay = 0.1;
                self.pos[1] += self.dir[1] * 0.0 + dy;
                self.count += 1;
            }
        } else {
            let c = self.count;
            self.count = c + 1;
            if c > 10 {
                self.count = 0;
            }
        }
        self.dots = (self.dots + 1) & 0xf;
    }
}

impl Vm {
    /// Frame of the loading animation as Canvas primitives (`self.hud`), on a screen
    /// cleared to black by the renderer (FRenderInterface::Clear).
    pub fn loading_animation_frame(&mut self, anim: &mut LoadingAnimation, width: u32, height: u32) {
        let (w, h) = (width as i32, height as i32);
        anim.step(w, h);
        self.hud.clear();
        self.viewport = (w, h);
        // Each cell drawn at its size, then shrunk by 4; below 0x33 alpha, removed, otherwise
        // 0x32 less alpha.
        let (drawn, text) = anim.draw_list();
        for s in drawn {
            let key = MATERIALS[s.material];
            let (us, vs) = self.material_size(&Value::Asset(key.split('|').nth(1).unwrap_or(key).to_string()));
            let (x1, y1) = (s.x - s.w * 0.5, s.y - s.w * 0.5);
            let tile = self.util_tile(x1, y1, x1 + s.w, y1 + s.h, [0.0, 0.0, us as f32, vs as f32], key, s.color);
            self.hud.push(tile);
        }
        // "Chargement" and its dots, CurX = ClipX − 240, CurY = ClipY − 65.
        let canvas = match self.canvas {
            Some(c) if self.objects[c].is_some() => c,
            _ => match self.class("Canvas") {
                Some(class) => {
                    let c = self.spawn(class);
                    self.canvas = Some(c);
                    c
                }
                None => return,
            },
        };
        self.canvas_update(canvas);
        self.set_prop(canvas, "CurX", Value::Float(w as f32 - 240.0));
        self.set_prop(canvas, "CurY", Value::Float(h as f32 - 65.0));
        self.set_prop(canvas, "DrawColor", Value::Struct("object.color".into(), [0x40u8, 0xc0, 0xff, 0xff].iter().map(|&b| Value::Byte(b)).collect()));
        self.set_prop(canvas, "Style", Value::Byte(1));
        let bytes: Vec<u8> = text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
        self.loading_print(canvas, FONT, &bytes);
    }
}

