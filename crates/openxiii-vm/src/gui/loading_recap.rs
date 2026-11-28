//! Improved build only: the wait behind a loading screen a mod holds (`hold_start()` in an
//! `on("loading")` handler, mod_script_hud.rs), such as the example mod that shows the console
//! version's recap bubbles between two maps. The window (the viewer's recap_step) does what each
//! step asks; this file only decides the order.

/// What the wait behind the held loading screen does next (see [`RecapFlow::step`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecapStep {
    /// The mod's loading screen on screen; `ready`: the level is fully prepared.
    Bubbles { ready: bool },
    /// The map file is read and the start is still held: the level is built and warmed up
    /// behind the loading screen (finish_travel, warm-up, graphics card idle), then the step is
    /// asked again.
    PrepareBehindBubbles,
    /// Released before the map was ready: the usual loading animation while it keeps loading.
    LoadingAnimation,
    /// The map file is read after the release: the level is built and warmed up behind the
    /// loading animation, then it starts by itself.
    PrepareThenStart,
    /// The level is prepared and the start released: it starts now.
    Start,
}

/// The wait behind a held loading screen, without the drawing or the loading. A release is
/// honoured at any moment: before the map is ready the loading animation takes the loading
/// screen's place and the level starts by itself once ready; while the start is held the level is
/// fully prepared behind the loading screen, then the mod is told (`on("loaded")`), and the level
/// starts at once when it releases the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecapFlow {
    /// The mod released the start (usually on a key, mouse button, touch or gamepad press).
    pub pressed: bool,
    /// The level is built and warmed up (PrepareBehindBubbles done).
    pub prepared: bool,
}

impl RecapFlow {
    /// The start released.
    pub fn press(&mut self) {
        self.pressed = true;
    }

    /// The next step, `loaded`: the background thread has read the map file.
    pub fn step(&self, loaded: bool) -> RecapStep {
        match (self.prepared, loaded, self.pressed) {
            (true, _, true) => RecapStep::Start,
            (true, _, false) => RecapStep::Bubbles { ready: true },
            (false, true, true) => RecapStep::PrepareThenStart,
            (false, true, false) => RecapStep::PrepareBehindBubbles,
            (false, false, true) => RecapStep::LoadingAnimation,
            (false, false, false) => RecapStep::Bubbles { ready: false },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Released before the map is ready: the loading animation at once (even on the very first
    /// step), then the level built behind it and started without a second release.
    #[test]
    fn press_before_load_goes_to_the_animation_then_starts_alone() {
        let mut f = RecapFlow::default();
        f.press();
        assert_eq!(f.step(false), RecapStep::LoadingAnimation);
        assert_eq!(f.step(false), RecapStep::LoadingAnimation);
        assert_eq!(f.step(true), RecapStep::PrepareThenStart);
        // Releasing again changes nothing.
        f.press();
        assert_eq!(f.step(false), RecapStep::LoadingAnimation);
    }

    /// Held: the loading screen while the map loads, the level prepared behind it once read,
    /// then the mod is told it is ready; a release then starts the level at once.
    #[test]
    fn no_press_prompts_only_once_prepared() {
        let mut f = RecapFlow::default();
        assert_eq!(f.step(false), RecapStep::Bubbles { ready: false });
        assert_eq!(f.step(true), RecapStep::PrepareBehindBubbles);
        // Map read but not prepared yet: not ready.
        assert_ne!(f.step(true), RecapStep::Bubbles { ready: true });
        f.prepared = true;
        assert_eq!(f.step(true), RecapStep::Bubbles { ready: true });
        assert_eq!(f.step(true), RecapStep::Bubbles { ready: true });
        f.press();
        assert_eq!(f.step(true), RecapStep::Start);
    }

    /// Released while the level was being prepared behind the loading screen (the event arrives
    /// once the preparation is done): the level starts, no animation in between.
    #[test]
    fn press_during_preparation_starts_right_after() {
        let mut f = RecapFlow::default();
        assert_eq!(f.step(true), RecapStep::PrepareBehindBubbles);
        f.prepared = true;
        f.press();
        assert_eq!(f.step(true), RecapStep::Start);
    }
}
