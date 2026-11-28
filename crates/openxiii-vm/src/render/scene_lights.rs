//! Lights of the Scene sub-path: real caches, one update per frame.
//! The Model+134 modes read their real actors or their static records again.
//! The snapshot is kept per pass before the following scenes; the old
//! compensating draws are no longer added to the real Inits and their RNG.
//! This does not cover the rest of Scene or RuntimeLightmaps/the global Renderer.

