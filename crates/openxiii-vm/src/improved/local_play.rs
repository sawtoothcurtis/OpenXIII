//! Local multiplayer and character presentation at the engine's extension points.
use crate::{
    value::{ObjId, Value},
    Vm,
};

pub(crate) const CAMPAIGN_AVAILABLE: bool = true;
pub(crate) const MAX_PLAYERS: u8 = crate::split_versus::MAX_LOCAL_PLAYERS;

pub(crate) fn character_preview_url(vm: &Vm) -> bool {
    vm.url.has_option(crate::character_screen::SCREEN_OPTION)
}

impl Vm {
    /// Back on the menus from a split-screen game (its in-game menu's Quit, among others):
    /// the Co-op page again, on its step after the way to play together (character_page.rs).
    pub(crate) fn reopen_coop_after_split_screen(&mut self) {
        if self.local_play().coop.is_some_and(|c| c.mode == crate::local_play::CoopMode::SplitScreen) {
            self.improved.character_screen.reopen_coop = Some(crate::local_play::CoopMode::SplitScreen);
        }
    }

    pub(crate) fn update_local_characters(&mut self, dt: f32) {
        self.character_screen_frame(dt);
        self.coop_looks_frame();
        self.coop_armor_frame();
        self.coop_carry_frame();
        self.coop_carry_pose_frame();
        self.coop_hostage_frame();
        self.coop_left_arm_frame();
    }
    /// Console command LOCALPLAYERS [n]: sets the players on this screen when a number follows,
    /// and answers the current number. `None` for any other command.
    pub(crate) fn local_players_exec(&mut self, word: &str, rest: &str) -> Option<String> {
        if word != "LOCALPLAYERS" {
            return None;
        }
        if let Ok(n) = rest.trim().parse::<u8>() {
            self.set_local_players(n);
        }
        Some(self.local_players().to_string())
    }

    /// Improved build: while XIIIMenuInGameMulti lays itself out (Created) and paints (Paint),
    /// the game counts one player. With more, the page takes the consoles' split-screen layout:
    /// a plain white dialog texture drawn at unscaled coordinates instead of the comic-strip
    /// bands, which on the split-screen menu frame (gui.rs) became an empty white square beside
    /// the buttons. With one, it is the page of a whole window, whose bands reach the edges of
    /// the player's view as the co-op pause menu's do (wide_menu.rs). Returns what to put back.
    pub(crate) fn versus_menu_one_player(&mut self) -> Option<(ObjId, Value)> {
        let game = self.level.and_then(|l| self.propr(l, "Game").as_object()).filter(|&g| self.objects.get(g).is_some_and(Option::is_some))?;
        let n = self.prop(game, "NumPlayers");
        (n.as_int() > 1).then(|| {
            self.set_prop(game, "NumPlayers", Value::Int(1));
            (game, n)
        })
    }
}
