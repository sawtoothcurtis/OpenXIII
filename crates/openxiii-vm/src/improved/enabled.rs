//! Modules and state installed by an improved VM.

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
macro_rules! register_modules { ($($items:item)*) => { $($items)* }; }
pub(crate) use register_modules;

#[derive(Default)]
pub(crate) struct State {
    /// Improved build: each player's held menu directions (stick state and repeats, menu_repeat.rs).
    pub(crate) pad_menu_repeat: Vec<(crate::menu_repeat::Directions, crate::menu_repeat::DirectionRepeat)>,
    /// Improved build only: the new boxes of the main menu (menu_boxes.rs).
    pub(crate) menu_boxes: crate::menu_boxes::MenuBoxes,
    /// Improved build only: the co-op character screen (character_screen.rs).
    pub(crate) character_screen: crate::character_screen::Screen,
    /// Improved build only: co-op players' vests and helmets fitted to their body (coop_armor.rs).
    pub(crate) coop_armor: crate::coop_armor::State,
    /// Improved build only: carried corpses seen by the other co-op players (coop_carry.rs).
    pub(crate) coop_carry: crate::coop_carry::State,
    /// Improved build only: the left arm of the players who carry a corpse (coop_carry_pose.rs).
    pub(crate) coop_carry_pose: crate::coop_carry_pose::State,
    /// Improved build only: co-op players' busy left arms (coop_left_arm.rs).
    pub(crate) coop_left_arm: crate::coop_left_arm::State,
    /// Improved build only: co-op hostages seen by the other players (coop_hostage.rs).
    pub(crate) coop_hostage: crate::coop_hostage::State,
    /// Improved build only: co-op players' bodies following their look (coop_look.rs).
    pub(crate) coop_look: crate::coop_look::State,
    /// Improved build only: the Controls page's Gamepad entry (controls_entry.rs).
    pub(crate) controls_entry: crate::controls_entry::ControlsEntry,
    /// Improved build only: the new lines of the Video options page (video_menu.rs).
    pub(crate) video_menu: crate::video_menu::VideoMenu,
    /// Improved build only: the in-game menus' Video line (pause_video.rs).
    pub(crate) pause_video: crate::pause_video::PauseVideo,
    /// Improved build only: the in-game menus' bands of the GUI frame being drawn (wide_menu.rs).
    pub(crate) wide_menu: Option<crate::wide_menu::WideMenu>,
    /// Improved build only: the menus' help bar under the mouse (help_bar_click.rs).
    pub(crate) help_bar: crate::help_bar_click::HelpBarClick,
    /// Improved build only: the menus' last input device, pages entered and clicks (menu_mouse.rs).
    pub(crate) menu_mouse: crate::menu_mouse::MenuMouse,
}
