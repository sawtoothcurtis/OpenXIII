//! Split-screen multiplayer on one PC (improved build only, off by default).
//!
//! The game's own split-screen multiplayer (the console versions' "Split screen" menu,
//! XIIIMenuSplitSetupClient) asks for one viewport per local player with
//! SetViewPortNumberForNextMap, then travels to a multiplayer map; the engine logs one player in
//! per viewport and the multiplayer scripts do the rest (starts, scores, respawn, one HUD and
//! scoreboard per viewport). The PC menus never offer it.
//!
//! Here the number of local players is a setting kept from map to map (`set_local_players`):
//! at 2, every standalone or hosted (LAN host) multiplayer map is played with one viewport per player, player 1 on
//! keyboard and mouse, player 2 on a gamepad (the gamepad handling of the co-op campaign,
//! coop.rs). Any other map (menu, campaign) has a single viewport. At 1 nothing changes.

use crate::value::ObjId;
use crate::Vm;

/// Most local players on one screen (the engine has four viewports).
pub const MAX_LOCAL_PLAYERS: u8 = 4;

impl Vm {
    /// Players on this screen for the multiplayer games started from now on: 1 (the original
    /// game) or 2 (split screen). Kept from map to map; read when a map logs its players in, so
    /// it is set before the travel to the multiplayer map. Improved build only: 1 otherwise.
    pub fn set_local_players(&mut self, n: u8) {
        let n = n.clamp(1, crate::improved::local_play::MAX_PLAYERS);
        self.engine.client.local_players = n;
    }

    /// Players on this screen for the multiplayer games (`set_local_players`).
    pub fn local_players(&self) -> u8 {
        self.engine.client.local_players.max(1)
    }

    /// The command line's number of players on this screen, once (the first map).
    pub fn apply_startup_local_players(&mut self, players: Option<u8>) {
        if let Some(n) = players.filter(|&n| n > 0) {
            self.set_local_players(n);
            // Kept by the game window for the next maps too (a co-op host on the network with two
            // players on this screen, local_play.rs).
            if n <= crate::local_play::MAX_LOCAL_PLAYERS {
                self.change_local_play(|p| p.set_local_players(n));
            }
        }
    }

    /// This map is played in split screen by several local players (co-op campaign or
    /// split-screen multiplayer).
    pub fn local_split(&self) -> bool {
        self.coop.on || self.coop.versus
    }

    /// Split screen, while a view is drawn: that view's player controller.
    pub(crate) fn split_view_controller(&self) -> Option<ObjId> {
        if !self.local_split() {
            return None;
        }
        let v = self.current_viewport.filter(|&v| self.objects.get(v).is_some_and(|o| o.is_some()))?;
        self.propr(v, "Actor").as_object().filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some()))
    }

    /// This map is a split-screen multiplayer game.
    pub fn versus_on(&self) -> bool {
        self.coop.versus
    }

    /// Before the players of a map log in (spawn_play_actors): with more than one player on
    /// this screen, a standalone or hosted (listen server) multiplayer game gets one viewport per player
    /// (SetViewPortNumberForNextMap N, as the split-screen menu does), any other map one.
    pub(crate) fn versus_before_logins(&mut self, game: ObjId) {
        self.coop.versus = false;
        let n = self.engine.client.local_players;
        if n <= 1 || self.coop.on {
            return;
        }
        // A standalone game (NM_Standalone) or a hosted one (NM_ListenServer: the other players
        // join from the network); never a dedicated server's map nor a client's.
        let local_game = self.level.is_some_and(|l| matches!(self.propr(l, "NetMode").as_byte(), 0 | 2));
        if local_game && self.is_a(game, "XIIIMPGameInfo") {
            self.engine.client.wanted = i32::from(n);
            self.coop.versus = true;
            self.log.push(format!("Log: Split screen: {n} players on this screen"));
        } else {
            self.engine.client.wanted = 1;
        }
    }
}

impl Vm {
    /// Pawn of local player `n` (1 for the first viewport), if alive in the world.
    fn versus_pawn(&self, n: usize) -> Option<ObjId> {
        let pc = *self.coop.players.get(n.checked_sub(1)?)?;
        self.objects.get(pc)?.as_ref()?;
        self.prop(pc, "Pawn").as_object().filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some()))
    }

    /// Testing: one line per local player (place, health, weapon, score and deaths).
    pub fn versus_report(&self) -> String {
        let mut out = Vec::new();
        for (i, &pc) in self.coop.players.iter().enumerate() {
            if self.objects.get(pc).is_none_or(|o| o.is_none()) {
                out.push(format!("P{} gone", i + 1));
                continue;
            }
            let pri = self.prop(pc, "PlayerReplicationInfo").as_object();
            let (score, deaths) = pri.map_or((0.0, 0.0), |r| (self.prop(r, "Score").as_float(), self.prop(r, "Deaths").as_float()));
            let state = self.objects[pc].as_ref().and_then(|o| o.state.clone()).unwrap_or_default();
            let pawn = self.versus_pawn(i + 1);
            let text = match pawn {
                Some(p) => {
                    let l = self.prop(p, "Location").as_vector();
                    let weapon = self
                        .prop(p, "Weapon")
                        .as_object()
                        .and_then(|w| self.objects[w].as_ref().map(|o| self.classes[o.class].name.clone()))
                        .unwrap_or_default();
                    let yaw = self.prop(pc, "Rotation").as_rotator()[1];
                    format!("at {:.0},{:.0},{:.0} yaw {yaw} health {} weapon {weapon}", l[0], l[1], l[2], self.prop(p, "Health").as_int())
                }
                None => "no pawn".to_string(),
            };
            out.push(format!("P{} {state} {text} score {score} deaths {deaths}", i + 1));
        }
        // Every player of the game (the scoreboard's list).
        let mut list = Vec::new();
        for &a in &self.actors {
            if self.objects.get(a).is_some_and(|o| o.is_some()) && self.is_a(a, "PlayerReplicationInfo") {
                let class = self.objects[a].as_ref().map(|o| self.classes[o.class].name.clone()).unwrap_or_default();
                list.push(format!("{class} {:?} {}", self.prop(a, "PlayerName"), self.prop(a, "Score").as_float()));
            }
        }
        out.push(format!("PRI [{}]", list.join(", ")));
        out.join(" | ")
    }

    /// Testing: TakeDamage(`damage`) on local player `to`'s pawn, caused by local player
    /// `from`'s pawn.
    pub fn versus_hit(&mut self, from: usize, to: usize, damage: i32) -> bool {
        let (Some(by), Some(target)) = (self.versus_pawn(from), self.versus_pawn(to)) else { return false };
        let l = self.prop(target, "Location").as_vector();
        let dt = self.class("DamageType");
        use crate::value::Value;
        self.event_pub(
            target,
            "TakeDamage",
            vec![Value::Int(damage), Value::Object(Some(by)), Value::vector(l[0], l[1], l[2]), Value::vector(0.0, 0.0, 0.0), Value::Class(dt)],
        );
        true
    }
}

impl Vm {
    /// Identities of the connected gamepads, in handle order (any number that stays the same for
    /// a gamepad while it is plugged in). With them, each player of a split screen keeps their own
    /// gamepad when another one is plugged in or removed. Without them, a handle is its identity.
    pub fn set_pad_ids(&mut self, ids: &[u64]) {
        if self.coop.pad_ids.as_deref() == Some(ids) {
            return;
        }
        self.coop.pad_ids = Some(ids.to_vec());
        // Without a split screen the original assignment (by handle) is left alone.
        if self.split_pads() {
            self.attrib_pad_to_viewport();
        }
    }

    /// Several players on this machine share it (one viewport each): the gamepads are shared out
    /// between them. A network co-op player alone on their machine has every gamepad.
    pub(crate) fn split_pads(&self) -> bool {
        // Lockstep co-op: one player per machine, every viewport of the session but one is
        // another machine's (whose gamepads come in that player's commands, lockstep.rs).
        if self.lockstep.is_some() {
            return false;
        }
        self.local_split() && self.engine.client.viewports.len() > 1
    }

    /// Gamepads of a split screen, players in viewport order (AttribPadToViewport). A player keeps
    /// the gamepad they hold while it stays connected (also from one map to the next). Gamepads
    /// nobody holds go to the players without one: in order when there are enough for all of them,
    /// otherwise to the last players, the first ones keeping the keyboard and mouse. So with one
    /// gamepad it is player 2's, with two the first is player 1's and the second player 2's.
    pub(crate) fn split_attrib_pads(&mut self, slots: &[u8]) {
        let present: Vec<usize> = (0..4).filter(|&h| self.pads.states[h].is_some()).collect();
        let ids: Vec<(usize, u64)> = match &self.coop.pad_ids {
            // Identities not yet given for this list (a map's first frame): nothing changes.
            Some(v) if v.len() != present.len() => return,
            Some(v) => present.iter().zip(v).map(|(&h, &i)| (h, i)).collect(),
            // No gamepad known yet on this map's new VM: nothing changes either.
            None if present.is_empty() => return,
            None => present.iter().map(|&h| (h, h as u64)).collect(),
        };
        let owners = &mut self.engine.client.pad_owners;
        owners.resize(slots.len(), None);
        for o in owners.iter_mut() {
            if o.is_some_and(|x| !ids.iter().any(|&(_, i)| i == x)) {
                *o = None;
            }
        }
        let free: Vec<u64> = ids.iter().map(|&(_, i)| i).filter(|i| !owners.contains(&Some(*i))).collect();
        let padless: Vec<usize> = (0..slots.len()).filter(|&r| owners[r].is_none()).collect();
        let start = padless.len().saturating_sub(free.len());
        for (&r, &id) in padless[start..].iter().zip(&free) {
            owners[r] = Some(id);
        }
        let handles: Vec<Option<usize>> = owners.iter().map(|o| o.and_then(|o| ids.iter().find(|&&(_, i)| i == o).map(|&(h, _)| h))).collect();
        for (&slot, h) in slots.iter().zip(handles) {
            self.engine.client.viewport_pads[usize::from(slot)] = h;
        }
    }

    /// Gamepads of the first player in a split screen (handles), the ones the host may read as
    /// that player's keyboard and mouse; `None` without a split screen (every gamepad).
    pub fn first_player_pads(&self) -> Option<Vec<usize>> {
        if !self.split_pads() {
            return None;
        }
        let slot = *self.engine.client.viewports.first()?;
        Some(self.engine.client.viewport_pads.get(usize::from(slot)).copied().flatten().into_iter().collect())
    }
}

impl Vm {
    /// The player of viewport `viewport` (a Player object) is the one with the keyboard and
    /// mouse: the first local player (or a viewport without a known player).
    pub(crate) fn split_mouse_player(&self, viewport: ObjId) -> bool {
        let actor = self.propr(viewport, "Actor").as_object();
        match (actor, self.coop.players.first()) {
            (Some(pc), Some(&first)) => pc == first,
            _ => true,
        }
    }

    /// Once per drawn view of a split-screen multiplayer game:
    /// - Start of the match: the game waits for every human player to fire (PlayerWaiting
    ///   ServerReStartPlayer sets bReadyToPlay, PendingMatch starts the match when all are
    ///   ready). On one screen, the first local player to fire starts it for all of them.
    /// - A player other than the first who has no gamepad is told to connect one.
    pub(crate) fn versus_frame(&mut self) {
        use crate::value::Value;
        let players: Vec<ObjId> = self.coop.players.iter().copied().filter(|&pc| self.objects.get(pc).is_some_and(|o| o.is_some())).collect();
        let game = self.level.and_then(|l| self.propr(l, "Game").as_object());
        if game.is_some_and(|g| self.prop(g, "bWaitingToStartMatch").as_bool()) {
            let pris: Vec<ObjId> = players.iter().filter_map(|&pc| self.prop(pc, "PlayerReplicationInfo").as_object()).collect();
            if pris.iter().any(|&r| self.prop(r, "bReadyToPlay").as_bool()) {
                for r in pris {
                    if !self.prop(r, "bReadyToPlay").as_bool() {
                        self.set_prop(r, "bReadyToPlay", Value::Bool(true));
                    }
                }
            }
        }
        if (0.0..HINT_EVERY).contains(&(self.level_time - self.coop.versus_hint_time)) {
            return;
        }
        self.coop.versus_hint_time = self.level_time;
        let text = gamepad_hint(&self.game_language());
        for (rank, &pc) in players.iter().enumerate().skip(1) {
            let has_pad = self.viewport_slot_of(pc).and_then(|s| self.engine.client.viewport_pads.get(usize::from(s)).copied().flatten()).is_some();
            if !has_pad {
                let line = text.replace("{n}", &(rank + 1).to_string());
                let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(line), Value::Name("None".into())]);
            }
        }
    }

    fn versus_menu_players_back(&mut self, kept: Option<(ObjId, crate::value::Value)>) {
        if let Some((game, n)) = kept {
            if self.objects.get(game).is_some_and(Option::is_some) {
                self.set_prop(game, "NumPlayers", n);
            }
        }
    }

    /// Script calls that split-screen multiplayer changes; `None`: as usual.
    /// The in-game menu of a multiplayer game (XIIIMenuInGameMulti.Created) gives "Quit game"
    /// to the first viewport only (HUD ViewPortId 0). Here every player can leave the match
    /// from their own half (the game then returns to the menu for everyone, as from the first
    /// player's menu, after the same "Are you sure?").
    pub(crate) fn versus_call(
        &mut self,
        f: crate::types::FuncId,
        this: ObjId,
        args: &[Option<crate::value::Value>],
    ) -> Option<crate::Result<(crate::value::Value, Vec<Option<crate::value::Value>>)>> {
        let function = &self.functions[f];
        let owner = self.classes[function.owner].name.clone();
        if function.name.eq_ignore_ascii_case("Paint") && owner.eq_ignore_ascii_case("XIIIMenuInGameMulti") && !self.coop.versus_menu {
            self.coop.versus_menu = true;
            let kept = self.versus_menu_one_player();
            let r = self.call(f, this, args.to_vec());
            self.versus_menu_players_back(kept);
            self.coop.versus_menu = false;
            return Some(r);
        }
        if function.name.eq_ignore_ascii_case("Created") && owner.eq_ignore_ascii_case("XIIIMenuInGameMulti") && !self.coop.versus_menu {
            self.coop.versus_menu = true;
            let kept = self.versus_menu_one_player();
            let r = self.call(f, this, args.to_vec());
            self.versus_menu_players_back(kept);
            self.coop.versus_menu = false;
            if let Some((hud, value)) = self.coop.versus_viewport_id.take() {
                self.set_prop(hud, "ViewPortId", value);
            }
            return Some(r);
        }
        if self.coop.versus_menu && function.name.eq_ignore_ascii_case("InitViewportId") && self.coop.versus_viewport_id.is_none() {
            let r = self.call_inner(f, this, args.to_vec());
            let id = self.prop(this, "ViewPortId");
            if id.as_int() != 0 {
                self.coop.versus_viewport_id = Some((this, id));
                self.set_prop(this, "ViewPortId", crate::value::Value::Int(0));
            }
            return Some(r);
        }
        None
    }
}

/// Seconds between two "connect a gamepad" hints.
const HINT_EVERY: f64 = 4.0;

/// "Player N: connect a gamepad" in the game's language (localization extension).
fn gamepad_hint(language: &str) -> &'static str {
    match language.to_ascii_lowercase().as_str() {
        "frt" => "Joueur {n} : branchez une manette",
        "det" => "Spieler {n}: Gamepad anschließen",
        "itt" => "Giocatore {n}: collega un gamepad",
        "est" => "Jugador {n}: conecta un mando",
        _ => "Player {n}: connect a gamepad",
    }
}

#[cfg(test)]
mod tests {
    use crate::Vm;

    #[test]
    fn screen_resolution_is_the_window_in_split_screen() {
        let mut vm = Vm::new();
        vm.engine.client.window_open = true;
        vm.client_size = (800, 600);
        vm.viewport = (800, 300);
        // One player: the viewport is the window, as in the original.
        assert_eq!(vm.windows_viewport_exec("GETCURRENTRES").as_deref(), Some("800x300"));
        // Split screen: the menus' CheckResolution must see the window, not the player's half
        // (below 640x480 it switched the window to 640x480).
        vm.coop.on = true;
        assert_eq!(vm.windows_viewport_exec("GETCURRENTRES").as_deref(), Some("800x600"));
        vm.coop.on = false;
        vm.coop.versus = true;
        assert_eq!(vm.windows_viewport_exec("GETCURRENTRES").as_deref(), Some("800x600"));
    }

    #[test]
    fn local_players_setting() {
        let mut vm = Vm::new();
        // Default: the original game, one player on this screen, no split screen.
        assert_eq!(vm.local_players(), 1);
        assert!(!vm.local_split());
        vm.set_local_players(2);
        assert_eq!(vm.local_players(), 2);
        // Kept with the engine from one map to the next.
        assert_eq!(vm.engine.client.local_players, 2);
        vm.set_local_players(9);
        assert_eq!(vm.local_players(), super::MAX_LOCAL_PLAYERS);
        vm.set_local_players(0);
        assert_eq!(vm.local_players(), 1);
        // The viewports themselves only change when a map logs its players in.
        assert_eq!(vm.engine.client.wanted, 1);
    }

    /// Gamepad handle of each of the two players after the host gives these gamepads.
    fn pads_after(vm: &mut Vm, ids: &[u64]) -> [Option<usize>; 2] {
        vm.set_pad_ids(ids);
        vm.set_pad_count(ids.len());
        let slots = vm.engine.client.viewports.clone();
        [0, 1].map(|r| vm.engine.client.viewport_pads[usize::from(slots[r])])
    }

    #[test]
    fn split_screen_gamepads() {
        let mut vm = Vm::new();
        vm.engine.client.wanted = 2;
        vm.engine.client.active_viewports();
        vm.coop.versus = true;
        // No gamepad: player 1 on keyboard and mouse, player 2 without controls.
        assert_eq!(pads_after(&mut vm, &[]), [None, None]);
        // One gamepad: player 2's.
        assert_eq!(pads_after(&mut vm, &[7]), [None, Some(0)]);
        // A second one plugged in: player 1's, player 2 keeps theirs.
        assert_eq!(pads_after(&mut vm, &[7, 9]), [Some(1), Some(0)]);
        // Player 1's removed: player 2 keeps theirs.
        assert_eq!(pads_after(&mut vm, &[7]), [None, Some(0)]);
        // Player 2's removed while player 1 has one: player 1 keeps it, no swap.
        assert_eq!(pads_after(&mut vm, &[7, 9]), [Some(1), Some(0)]);
        assert_eq!(pads_after(&mut vm, &[9]), [Some(0), None]);
        // Plugged back: player 2's again.
        assert_eq!(pads_after(&mut vm, &[9, 4]), [Some(0), Some(1)]);
        assert_eq!(vm.first_player_pads(), Some(vec![0]));
        // Two gamepads from the start: the first is player 1's, the second player 2's.
        vm.engine.client.pad_owners.clear();
        vm.coop.pad_ids = None;
        assert_eq!(pads_after(&mut vm, &[3, 5]), [Some(0), Some(1)]);
        // A new map (new VM state): nothing known before the host gives the gamepads.
        vm.coop.pad_ids = None;
        vm.set_pad_count(0);
        assert_eq!(vm.engine.client.pad_owners, vec![Some(3), Some(5)]);
        // Co-op with a single viewport here (network co-op): every gamepad, as without co-op.
        vm.engine.client.wanted = 1;
        vm.engine.client.active_viewports();
        vm.coop.on = true;
        assert_eq!(vm.first_player_pads(), None);
        vm.coop.on = false;
        vm.engine.client.wanted = 2;
        vm.engine.client.active_viewports();
        // Without a split screen: the original assignment, in order, and every gamepad for the host.
        vm.coop.versus = false;
        assert_eq!(pads_after(&mut vm, &[5, 3]), [Some(0), Some(1)]);
        assert_eq!(vm.first_player_pads(), None);
    }
}
