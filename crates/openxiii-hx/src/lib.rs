//! Ubisoft's HX sound engine, as XIII uses it (HXAudio.dll): sound programs and
//! their interpreter, sound objects (waves, programs, actors), variables, timers, listeners.
//!
//! The engine plays nothing itself: it holds [`engine::Voice`]s (wave, gain, pitch,
//! pan, position) that a mixer makes audible.

pub mod bank;
mod diagnostics;
pub mod engine;
pub mod interp;
pub mod listener;
pub mod mem;
pub mod program;
pub mod rand;
pub mod stream;
pub mod subsystem;

pub use engine::Engine;
