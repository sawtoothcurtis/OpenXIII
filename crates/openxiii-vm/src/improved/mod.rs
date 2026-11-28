//! The VM edition is selected once. Both editions provide the same internal extension points.
#[cfg(feature = "improved")]
#[path = "enabled.rs"]
mod edition;
#[cfg(not(feature = "improved"))]
#[path = "../faithful/mod.rs"]
mod edition;
pub(crate) use edition::*;

// Both the menus and standalone Lua mods can read the game language.
#[cfg(any(feature = "improved", feature = "mods"))]
#[path = "../engine/text_language.rs"]
mod text_language;
