//! Lockstep co-op: the periodic full fingerprint, a detection of differences the light
//! per-step fingerprint (random number, time, players' and actors' places) cannot see: every
//! property and the script state of every object (inventories and ammunition, every pawn's
//! health, the AI controllers' states and targets, the game's script variables), in five
//! parts so that a difference says where it is. It is computed in `PERIOD` slices, one per
//! step (no frame pays for the whole game at once), and compared with the other machine's
//! once per period (about every 5 s). Detection only: nothing is repaired.
//!
//! What only drawing writes is left out: the HUD and its messages, the Canvas, the viewports
//! (each machine's window), the drawing-only objects (shadows), each actor's LastRenderTime and
//! LastRenderDist, and LevelInfo's date fields.

use crate::value::{ObjId, Value};
use crate::Vm;

/// Steps of one full fingerprint (about 5 s at 60 steps a second).
pub const PERIOD: u64 = 300;

/// The parts of the full fingerprint (index in its values).
pub const PARTS: [&str; 5] = ["pawns", "controllers", "inventory", "other actors", "other objects"];

/// The full fingerprint being made, then made.
#[derive(Default, Debug, Clone)]
pub struct FullPrint {
    /// Objects of the period (their number at its first step, fixed for the period).
    count: usize,
    acc: [u64; 5],
    /// Time spent making it (seconds), for the session's measurements.
    pub seconds: f64,
}

const BASIS: u64 = 0xcbf2_9ce4_8422_2325;

#[inline]
fn eat(h: &mut u64, b: &[u8]) {
    for &x in b {
        *h = (*h ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3);
    }
}

fn fold(h: u64) -> u32 {
    (h ^ (h >> 32)) as u32
}

impl Vm {
    /// Step `step`'s slice of the full fingerprint; at the period's last step, the period's
    /// number and its full fingerprint (each part folded to 32 bits).
    pub fn lockstep_full_slice(&mut self, step: u64) -> Option<(u64, [u32; 5])> {
        let t = std::time::Instant::now();
        let k = step % PERIOD;
        let mut fp = self.lockstep.as_ref().map(|l| l.full.clone()).unwrap_or_default();
        if k == 0 {
            fp = FullPrint { count: self.objects.len(), acc: [BASIS; 5], seconds: fp.seconds };
        }
        let skip = false;
        let (from, to) = ((fp.count as u64 * k / PERIOD) as usize, (fp.count as u64 * (k + 1) / PERIOD) as usize);
        if !skip {
            for o in from..to.min(self.objects.len()) {
                let Some(part) = self.full_part(o) else { continue };
                let mut h = BASIS;
                eat(&mut h, &(o as u32).to_le_bytes());
                self.full_object(&mut h, o);
                // Order-independent within the part: the slices may be met in any order.
                fp.acc[part] = fp.acc[part].wrapping_add(h);
            }
        }
        fp.seconds += t.elapsed().as_secs_f64();
        let done = (k == PERIOD - 1).then(|| (step / PERIOD, fp.acc.map(fold)));
        if let Some(l) = self.lockstep.as_mut() {
            l.full = fp;
        }
        done
    }

    /// The part object `o` belongs to; None: not game state (or gone).
    fn full_part(&self, o: ObjId) -> Option<usize> {
        let x = self.objects.get(o)?.as_ref()?;
        let class = &self.classes[x.class];
        if self.is_a(o, "HUD") || self.is_a(o, "Canvas") || self.is_a(o, "Player") || class.name.to_ascii_lowercase().starts_with("hud") || self.lockstep_drawing_only(o) {
            return None;
        }
        Some(if self.is_a(o, "Pawn") {
            0
        } else if self.is_a(o, "Controller") {
            1
        } else if self.is_a(o, "Inventory") {
            2
        } else if self.is_a(o, "Actor") {
            3
        } else {
            4
        })
    }

    /// One object: name, class, state, state code position and latent action, every property
    /// but those only drawing or the PC's clock write.
    fn full_object(&self, h: &mut u64, o: ObjId) {
        let Some(x) = self.objects.get(o).and_then(|x| x.as_ref()) else { return };
        let s = |h: &mut u64, t: &str| {
            eat(h, &(t.len() as u32).to_le_bytes());
            eat(h, t.as_bytes());
        };
        s(h, &x.name);
        s(h, &self.classes[x.class].name);
        s(h, x.state.as_deref().unwrap_or(""));
        if let Some(t) = self.threads.get(&o) {
            eat(h, &(t.pc as u64).to_le_bytes());
            eat(h, &[t.latent as u8, u8::from(t.restart)]);
        }
        let class = &self.classes[x.class];
        let mut skip: Vec<usize> = ["lastrendertime", "lastrenderdist"].iter().filter_map(|n| class.prop_index.get(*n).copied()).collect();
        if Some(o) == self.level {
            skip.extend(["year", "month", "dayofweek", "day", "hour", "minute", "second", "millisecond"].iter().filter_map(|n| class.prop_index.get(*n).copied()));
        }
        for (i, v) in x.props.iter().enumerate() {
            if !skip.contains(&i) {
                self.full_value(h, v);
            }
        }
    }

    fn full_value(&self, h: &mut u64, v: &Value) {
        match v {
            Value::Void => eat(h, &[0]),
            Value::Byte(b) => eat(h, &[1, *b]),
            Value::Int(i) => {
                eat(h, &[2]);
                eat(h, &i.to_le_bytes());
            }
            Value::Bool(b) => eat(h, &[3, u8::from(*b)]),
            Value::Float(f) => {
                eat(h, &[4]);
                eat(h, &f.to_bits().to_le_bytes());
            }
            Value::Name(n) => {
                eat(h, &[5]);
                eat(h, n.as_str().as_bytes());
            }
            Value::Str(t) => {
                eat(h, &[6]);
                eat(h, t.as_bytes());
            }
            // References by name: object numbers may differ in order without any effect.
            Value::Object(o) => {
                eat(h, &[7]);
                match o.and_then(|o| self.objects.get(o).and_then(|x| x.as_ref())) {
                    Some(x) => eat(h, x.name.as_bytes()),
                    None => eat(h, &[0xff]),
                }
            }
            Value::Asset(a) => {
                eat(h, &[8]);
                eat(h, a.as_bytes());
            }
            Value::Class(c) => {
                eat(h, &[9]);
                if let Some(c) = c {
                    eat(h, self.classes[*c].name.as_bytes());
                }
            }
            Value::Struct(n, f) => {
                eat(h, &[10]);
                eat(h, n.as_str().as_bytes());
                for x in f.iter() {
                    self.full_value(h, x);
                }
            }
            Value::Array(items) => {
                eat(h, &[11]);
                eat(h, &(items.len() as u32).to_le_bytes());
                for x in items {
                    self.full_value(h, x);
                }
            }
            Value::Delegate(o, n) => {
                eat(h, &[12]);
                if let Some(x) = o.and_then(|o| self.objects.get(o).and_then(|x| x.as_ref())) {
                    eat(h, x.name.as_bytes());
                }
                eat(h, n.as_str().as_bytes());
            }
            Value::Vector(x) => {
                eat(h, &[13]);
                for c in x {
                    eat(h, &c.to_bits().to_le_bytes());
                }
            }
            Value::Rotator(r) => {
                eat(h, &[14]);
                for c in r {
                    eat(h, &c.to_le_bytes());
                }
            }
        }
    }
}

/// The first part two full fingerprints disagree on.
pub fn first_difference(a: &[u32; 5], b: &[u32; 5]) -> Option<&'static str> {
    (0..5).find(|&i| a[i] != b[i]).map(|i| PARTS[i])
}

#[cfg(test)]
mod tests {
    use super::*;

}
