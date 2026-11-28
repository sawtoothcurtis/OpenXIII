//! The faithful VM has no improved state.

#[path = "canvas.rs"]
pub(crate) mod canvas;
#[path = "graphics.rs"]
pub(crate) mod graphics;
#[path = "gui.rs"]
pub(crate) mod gui;
#[path = "hooks.rs"]
pub(crate) mod hooks;
#[path = "input.rs"]
pub(crate) mod input;
#[path = "local_play.rs"]
pub(crate) mod local_play;
#[path = "navigation.rs"]
pub(crate) mod navigation;
#[path = "network.rs"]
pub(crate) mod network;
#[path = "presentation.rs"]
pub(crate) mod presentation;
#[path = "storage.rs"]
pub(crate) mod storage;
#[path = "visual_effects.rs"]
pub(crate) mod visual_effects;

// Keep the logical paths of edition-specific public modules and tests.
macro_rules! register_modules { ($($items:item)*) => {  }; }
pub(crate) use register_modules;

#[derive(Default)]
pub(crate) struct State;
