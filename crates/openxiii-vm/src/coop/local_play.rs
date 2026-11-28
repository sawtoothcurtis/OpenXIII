//! How the players in front of this screen want to play: the one interface between the menus
//! (improved build: the Co-op page, coop_page.rs, and the multiplayer pages' "Players on this
//! screen" setting) and the game modes that read it when a map starts.
//!
//! The game window keeps the reference copy (`App` in the viewer), because it outlives every map;
//! each map's VM gets that copy when it is made ([`Vm::set_local_play`]) and hands back what the
//! menus changed ([`Vm::take_local_play_change`]).
//!
//! Reading it, for a game mode:
//! - [`LocalPlay::coop`] is `Some` from the moment a way to play together is chosen on the Co-op
//!   page until the Co-op page is left (Escape on its first step) or the menu map is loaded
//!   again (back to the main menu): every map loaded in between, travels included, belongs to
//!   that co-op session. [`CoopSession::mode`] says how the players are connected,
//!   [`CoopSession::start`] which of the menu's ways of starting a game launched it.
//! - [`LocalPlay::local_players`] is the number of players on this screen for the multiplayer
//!   games (Host LAN, Join LAN, Bot challenge): 1 or 2.
//!
//! Nothing in this module changes the game: without a game mode reading it, the solo and
//! multiplayer games are the original ones.

use crate::Vm;

/// How the co-op players are connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoopMode {
    /// Two players on this computer, one half of the screen each.
    SplitScreen,
    /// This computer hosts a co-op campaign that a player on the local network joins.
    LanHost,
    /// This computer joins a co-op campaign hosted on the local network.
    LanJoin,
}

/// Which way of starting a game the co-op session used (the menu's own flows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoopStart {
    /// The new-game video, then the first level.
    NewGame,
    /// The Continue page (last save).
    Continue,
    /// The Load page (a save slot).
    Load,
    /// The Select level page (a mission start or a checkpoint, then the equipment page).
    SelectLevel,
}

/// A co-op session chosen on the Co-op page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoopSession {
    pub mode: CoopMode,
    /// None until a way of starting the game is chosen (and always None to join: the host
    /// chooses the game).
    pub start: Option<CoopStart>,
}

/// Players on this screen allowed by the multiplayer pages' setting.
pub const MAX_LOCAL_PLAYERS: u8 = 2;

/// What the players in front of this screen chose (see the module's documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalPlay {
    pub coop: Option<CoopSession>,
    /// 1 or 2 (see [`LocalPlay::set_local_players`]).
    pub local_players: u8,
    /// The characters the co-op players chose on the character screen (improved build,
    /// character_screen.rs): (roster index, variant) of the first and second player of this
    /// computer. None until chosen; cleared with the session.
    pub characters: Option<[(u8, u8); 2]>,
}

impl Default for LocalPlay {
    fn default() -> Self {
        LocalPlay { coop: None, local_players: 1, characters: None }
    }
}

impl LocalPlay {
    /// Sets the number of players on this screen, kept between 1 and [`MAX_LOCAL_PLAYERS`].
    pub fn set_local_players(&mut self, n: u8) {
        self.local_players = n.clamp(1, MAX_LOCAL_PLAYERS);
    }

    /// The co-op session in split screen (the local co-op mode).
    pub fn split_screen(&self) -> bool {
        self.coop.is_some_and(|s| s.mode == CoopMode::SplitScreen)
    }

    /// One line for the log.
    pub fn describe(&self) -> String {
        let coop = match self.coop {
            None => "off".to_string(),
            Some(s) => {
                let mode = match s.mode {
                    CoopMode::SplitScreen => "split-screen",
                    CoopMode::LanHost => "LAN host",
                    CoopMode::LanJoin => "LAN join",
                };
                let start = match s.start {
                    None => "none",
                    Some(CoopStart::NewGame) => "new game",
                    Some(CoopStart::Continue) => "continue",
                    Some(CoopStart::Load) => "load",
                    Some(CoopStart::SelectLevel) => "select level",
                };
                format!("{mode}, started by {start}")
            }
        };
        let characters = match self.characters {
            Some(c) => format!(", characters {}.{} and {}.{}", c[0].0, c[0].1, c[1].0, c[1].1),
            None => String::new(),
        };
        format!("co-op {coop}, players on this screen {}{characters}", self.local_players)
    }
}

/// The VM's copy and whether the menus changed it since the game window last took it.
#[derive(Default)]
pub(crate) struct LocalPlayCopy {
    pub(crate) value: LocalPlay,
    pub(crate) changed: bool,
}

impl Vm {
    /// The game window's copy, given to this map's VM.
    pub fn set_local_play(&mut self, value: LocalPlay) {
        self.local_play.value = value;
        self.local_play.changed = false;
    }

    /// What the players chose (as the game window gave it, with the menus' changes).
    pub fn local_play(&self) -> LocalPlay {
        self.local_play.value
    }

    /// The new value when the menus changed it since the last call.
    pub fn take_local_play_change(&mut self) -> Option<LocalPlay> {
        std::mem::take(&mut self.local_play.changed).then_some(self.local_play.value)
    }

    /// A change made by the menus.
    pub(crate) fn change_local_play(&mut self, f: impl FnOnce(&mut LocalPlay)) {
        let before = self.local_play.value;
        f(&mut self.local_play.value);
        // The characters belong to the session: none without one, none chosen yet in a new one.
        if self.local_play.value.coop.map(|s| s.mode) != before.coop.map(|s| s.mode) {
            self.local_play.value.characters = None;
        }
        if self.local_play.value != before {
            self.local_play.changed = true;
        }
    }

    /// A map starts (UGameEngine::LoadMap): the menu's own map ends a co-op session (back to the
    /// main menu); any other map logs what it starts with when a co-op session is on.
    pub fn local_play_map_start(&mut self, map: &str) {
        if map.to_ascii_lowercase().starts_with("mapmenu") {
            self.reopen_coop_after_split_screen();
            self.change_local_play(|p| p.coop = None);
        } else if self.local_play.value.coop.is_some() {
            println!("map {map}: {}", self.local_play.value.describe());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_limits() {
        let mut p = LocalPlay::default();
        assert_eq!((p.coop, p.local_players), (None, 1));
        p.set_local_players(2);
        assert_eq!(p.local_players, 2);
        p.set_local_players(0);
        assert_eq!(p.local_players, 1);
        p.set_local_players(9);
        assert_eq!(p.local_players, MAX_LOCAL_PLAYERS);
        assert!(!p.split_screen());
        p.coop = Some(CoopSession { mode: CoopMode::SplitScreen, start: Some(CoopStart::SelectLevel) });
        assert!(p.split_screen());
        assert_eq!(p.describe(), "co-op split-screen, started by select level, players on this screen 2");
    }

    #[test]
    fn changes_are_handed_back_once_and_the_menu_map_ends_the_session() {
        let mut vm = Vm::default();
        assert_eq!(vm.take_local_play_change(), None);
        vm.change_local_play(|p| p.coop = Some(CoopSession { mode: CoopMode::SplitScreen, start: None }));
        let got = vm.take_local_play_change().and_then(|p| p.coop).map(|s| s.mode);
        assert_eq!(got, Some(CoopMode::SplitScreen));
        assert_eq!(vm.take_local_play_change(), None, "taken once");
        vm.change_local_play(|p| p.coop = Some(CoopSession { mode: CoopMode::SplitScreen, start: None }));
        assert_eq!(vm.take_local_play_change(), None, "no change, nothing handed back");
        vm.local_play_map_start("Hual01a");
        assert!(vm.local_play().split_screen(), "a level keeps the session");
        vm.local_play_map_start("MapMenu");
        assert_eq!(vm.local_play().coop, None);
        assert_eq!(vm.take_local_play_change().map(|p| p.coop), Some(None), "the window learns the session ended");
        // The window's copy replaces the VM's without counting as a change.
        vm.set_local_play(LocalPlay { coop: None, local_players: 2, characters: None });
        assert_eq!((vm.local_play().local_players, vm.take_local_play_change()), (2, None));
    }

    #[cfg(feature = "improved")]
    #[test]
    fn leaving_a_split_screen_game_for_the_menu_opens_the_coop_page_again() {
        let mut vm = Vm::default();
        vm.change_local_play(|p| p.coop = Some(CoopSession { mode: CoopMode::LanHost, start: None }));
        vm.local_play_map_start("MapMenu");
        assert_eq!(vm.improved.character_screen.reopen_coop, None, "only after a split-screen game");
        vm.change_local_play(|p| p.coop = Some(CoopSession { mode: CoopMode::SplitScreen, start: Some(CoopStart::NewGame) }));
        vm.local_play_map_start("Hual01a");
        assert_eq!(vm.improved.character_screen.reopen_coop, None, "a level keeps the game");
        vm.local_play_map_start("MapMenu");
        assert_eq!(vm.improved.character_screen.reopen_coop, Some(CoopMode::SplitScreen));
        assert_eq!(vm.local_play().coop, None);
    }
}
