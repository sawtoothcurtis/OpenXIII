// Improved modules retain their crate-root paths for callers and saved test inventories.
/// Improved build only: the main menu's Mods box and the Load page's Select level entry.
#[path = "../gui/menu_boxes.rs"]
pub mod menu_boxes;
/// Improved build only: texts added to the menus and the loading recap, in each language of the game.
#[path = "../gui/added_text.rs"]
pub mod added_text;
/// Improved build only: still frames of the game's videos for the menus, made in the background.
#[path = "../gui/still_frames.rs"]
pub mod still_frames;
/// Improved build only: the Gamepad entry of the Controls page (gamepad settings).
#[path = "../gui/controls_entry.rs"]
pub mod controls_entry;
/// Improved build only: the new lines of the Video options page.
#[path = "../gui/video_menu.rs"]
pub mod video_menu;
/// Improved build only (Graphics: Enhanced): the in-game menu's Video line.
#[path = "../gui/pause_video.rs"]
pub mod pause_video;
/// Improved build only: the in-game menus' bands to the edges of a wide window.
#[path = "../gui/wide_menu.rs"]
pub(crate) mod wide_menu;
/// Improved build only: the menus' help bar answers the mouse.
#[path = "../gui/help_bar_click.rs"]
pub(crate) mod help_bar_click;
/// Improved build only: the menus follow the device last used.
#[path = "../gui/menu_mouse.rs"]
pub(crate) mod menu_mouse;
/// Improved build only: the pictures of the Select level page (Jones, Cine14 background).
#[path = "../gui/select_level_art.rs"]
pub mod select_level_art;
/// Improved build only: the menus' cut-out characters with their outer edge redrawn.
#[path = "../gui/menu_figures.rs"]
pub mod menu_figures;
/// Improved build only: the Select level page (levels list, then equipment).
#[path = "../gui/select_level_page.rs"]
pub mod select_level_page;
/// Improved build only: the Co-op page and the multiplayer pages' players setting.
#[path = "../coop/coop_page.rs"]
pub mod coop_page;
#[path = "../net/multi_join.rs"]
pub mod multi_join;
/// Improved build only: the co-op character screen (a paused level, a fixed camera per player).
#[path = "../gui/character_screen.rs"]
pub mod character_screen;
/// Improved build only: the character screen's page and its place in the co-op flow.
#[path = "../gui/character_page.rs"]
pub mod character_page;
#[path = "../coop/coop_save.rs"]
pub mod coop_save;
/// Improved build only: the co-op players' chosen characters in the game (body, arms).
#[path = "../coop/coop_looks.rs"]
pub mod coop_looks;
/// Improved build with mods only: a co-op player who is not XIII shouts with his character's voice.
#[cfg(feature = "mods")]
#[path = "../coop/coop_voices.rs"]
pub mod coop_voices;
#[cfg(all(test, feature = "mods"))]
#[path = "../../tests/unit/coop/coop_voices_test.rs"]
mod coop_voices_test;
/// Improved build only: the vest and the helmet fitted to a co-op player's character.
#[path = "../coop/coop_armor.rs"]
pub mod coop_armor;
/// Improved build only: a corpse carried by a co-op player, seen by the other players.
#[path = "../coop/coop_carry.rs"]
pub mod coop_carry;
/// Improved build only: the carrier of a corpse holds it with the left arm, seen by the others.
#[path = "../coop/coop_carry_pose.rs"]
pub mod coop_carry_pose;
/// Improved build only: the busy left arm of a co-op player's full body.
#[path = "../coop/coop_left_arm.rs"]
pub mod coop_left_arm;
/// Improved build only: the hostage a co-op player holds, seen by the other players.
#[path = "../coop/coop_hostage.rs"]
pub mod coop_hostage;
/// Improved build only: the full body of a co-op player follows their look head first.
#[path = "../coop/coop_look.rs"]
pub mod coop_look;
/// Improved build only: the console version's recap bubbles at the start of a map load.
#[path = "../gui/loading_recap.rs"]
pub mod loading_recap;
/// Improved build only: a gamepad direction held in a menu repeats like a held arrow key.
#[path = "../gui/menu_repeat.rs"]
pub mod menu_repeat;
/// Improved build only: the original menu sounds for the pages OpenXIII draws itself.
#[path = "../gui/menu_sounds.rs"]
pub mod menu_sounds;
/// Improved build only: a co-op player's body with another character's mesh plays every gesture.
#[path = "../coop/coop_body_anims.rs"]
pub mod coop_body_anims;
/// Improved build only: a co-op player holding an object stops drawing its weapon.
#[path = "../coop/coop_item_gesture.rs"]
mod coop_item_gesture;
#[cfg(test)]
#[path = "../../tests/unit/gui/character_page_close_test.rs"]
mod character_page_close_test;
