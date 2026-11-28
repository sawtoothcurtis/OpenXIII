//! Chat in lockstep co-op (improved build): each player types their message on their own machine,
//! as a network client's console does, and only the finished line crosses the network.
//!
//! Every machine runs the whole split-screen simulation, which has a single console for all its
//! viewports (InteractionMaster.Console). Left to the simulation, a player's Talk key and typed
//! characters went into that shared console on both machines: each player saw the other's line
//! being typed as their own, and the line was then said by the first player (the console's
//! ViewportOwner). Here the local player's Talk / TeamTalk key opens a line kept by this machine
//! only ([`ChatLine`]), edited as Engine.Console's Typing state edits TypedStr; Enter puts the
//! line in the player's next command ([`crate::lockstep::PlayerCmd::say`]), and every machine
//! runs it at the same step as that player's console command (UInteraction::execConsoleCommand
//! with the typer as the console's owner). The broadcast and the HUD messages ("Name: text")
//! are then the game's own, as in multiplayer. While the line is open it is drawn on this
//! machine's view only, by the game's console code (XIIIConsole's Typing PostRender).

use crate::player_input::{IST_PRESS, IST_TYPE};
use crate::value::{ObjId, Value};
use crate::Vm;

/// Console.MaxHistory.
const HISTORY: usize = 16;
/// Longest line Console.Typing.KeyType accepts (Len(TypedStr) < 240).
const LINE_MAX: usize = 240;

/// This machine's player's chat line and its history (Engine.Console's TypedStr, History,
/// HistoryTop/Bot/Cur, bSkipNextKey), never part of the simulation.
#[derive(Debug, Clone)]
pub struct ChatLine {
    /// The line is open (Console's Typing state).
    pub open: bool,
    /// TypedStr, starting with "Say " or "TeamSay ".
    pub typed: String,
    skip_next: bool,
    history: [String; HISTORY],
    top: usize,
    bot: Option<usize>,
    cur: usize,
}

impl Default for ChatLine {
    fn default() -> Self {
        ChatLine { open: false, typed: String::new(), skip_next: false, history: Default::default(), top: 0, bot: None, cur: 0 }
    }
}

/// What a key or character did to the line.
#[derive(Debug, PartialEq)]
pub enum ChatEffect {
    /// Not the line's: the event goes on to the game.
    Pass,
    /// Taken by the line.
    Taken,
    /// Enter on a line: the command to run (TypedStr), the line closed.
    Send(String),
}

impl ChatLine {
    /// Console.Talk / Console.TeamTalk: the line opens with "Say " or "TeamSay ", and the
    /// character of the key that opened it is skipped.
    pub fn open(&mut self, team: bool) {
        self.skip_next = true;
        self.typed = if team { "TeamSay ".into() } else { "Say ".into() };
        self.open = true;
        self.cur = self.top;
    }

    fn close(&mut self) {
        self.open = false;
    }

    /// Console.Typing.KeyEvent for an event of the open line (EInputKey, EInputAction).
    pub fn key_event(&mut self, key: u8, action: u8) -> ChatEffect {
        if !self.open {
            return ChatEffect::Pass;
        }
        // Escape: the line emptied first, closed when it is empty.
        if key == 27 {
            if !self.typed.is_empty() {
                self.typed.clear();
                self.cur = self.top;
            } else {
                self.close();
            }
            return ChatEffect::Taken;
        }
        // A release, an axis, a hold: not the line's (keys held before it opened are let go).
        if action != IST_PRESS {
            return ChatEffect::Pass;
        }
        match key {
            13 => {
                self.close();
                if self.typed.is_empty() {
                    return ChatEffect::Taken;
                }
                let line = std::mem::take(&mut self.typed);
                self.history[self.top] = line.clone();
                self.top = (self.top + 1) % HISTORY;
                if self.bot.is_none_or(|b| b == self.top) {
                    self.bot = Some(self.bot.map_or(0, |b| (b + 1) % HISTORY));
                }
                self.cur = self.top;
                ChatEffect::Send(line)
            }
            // Up: the previous line of the history.
            38 => {
                if let Some(bot) = self.bot {
                    self.cur = if self.cur == bot { self.top } else { (self.cur + HISTORY - 1) % HISTORY };
                    self.typed = self.history[self.cur].clone();
                }
                ChatEffect::Taken
            }
            // Down: the next one.
            40 => {
                if let Some(bot) = self.bot {
                    self.cur = if self.cur == self.top { bot } else { (self.cur + 1) % HISTORY };
                    self.typed = self.history[self.cur].clone();
                }
                ChatEffect::Taken
            }
            // Backspace, Left: the last character removed.
            8 | 37 => {
                self.typed.pop();
                ChatEffect::Taken
            }
            // Any other press is the line's while it is open.
            _ => ChatEffect::Taken,
        }
    }

    /// Console.Typing.KeyType for a typed character (WM_CHAR) of the open line.
    pub fn key_type(&mut self, key: u8) -> ChatEffect {
        if !self.open {
            return ChatEffect::Pass;
        }
        if std::mem::take(&mut self.skip_next) {
            return ChatEffect::Taken;
        }
        if key >= 32 && key != b'~' && key != b'`' && self.typed.chars().count() < LINE_MAX {
            self.typed.push(char::from(key));
            return ChatEffect::Taken;
        }
        // Refused by the console: the original lets it go on to the other interactions, which
        // a line typed during play leaves to nobody.
        ChatEffect::Taken
    }
}

/// The command a key's binding runs opens the chat line: Some(team) for Talk / TeamTalk.
pub fn talk_binding(command: &str) -> Option<bool> {
    let c = command.trim();
    if c.eq_ignore_ascii_case("Talk") {
        Some(false)
    } else if c.eq_ignore_ascii_case("TeamTalk") {
        Some(true)
    } else {
        None
    }
}

/// Each player's name in the chat (player order): their co-op name, None without one; a name
/// already taken by a player before them gets " 2", " 3"... (the same name on both PCs).
pub fn chat_names(names: &[Option<String>]) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = Vec::new();
    for n in names {
        let n = n.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let given = n.map(|n| {
            let taken = |c: &str| out.iter().flatten().any(|o| o.eq_ignore_ascii_case(c));
            if !taken(&n) {
                return n;
            }
            (2..).map(|k| format!("{n} {k}")).find(|c| !taken(c)).unwrap_or(n)
        });
        out.push(given);
    }
    out
}

/// The console's fields swapped while this machine draws its own open line.
#[derive(Debug, Clone)]
pub(crate) struct DrawSwap {
    console: ObjId,
    state: Option<String>,
    state_class: Option<crate::value::ClassId>,
    typed: Value,
    visible: Value,
}

impl Vm {
    /// The simulation's console (InteractionMaster.Console, shared by every viewport).
    fn lockstep_console(&self) -> Option<ObjId> {
        let master = self.interaction_master?;
        self.propr(master, "Console").as_object().filter(|&c| self.objects.get(c).is_some_and(|o| o.is_some()))
    }

    /// This machine's player `pc` gives its keys to the game's bindings now: no menu, no
    /// message box, no console line of the simulation has the input.
    fn lockstep_chat_may_open(&self, pc: ObjId) -> bool {
        if self.propr(pc, "bIsInMenu").as_bool() || self.coop_menu_open(pc) || self.coop_end_menu_open(pc) {
            return false;
        }
        !self.lockstep_console().is_some_and(|c| self.propr(c, "bTyping").as_bool())
    }

    /// This machine's player's events of the next command (`events`, in order) through its chat
    /// line: the Talk / TeamTalk key opens it, the open line takes the presses and the typed
    /// characters. What is left goes to the game; `say` is the line sent, if any.
    pub(crate) fn lockstep_chat_filter(&mut self, events: Vec<(u8, u8, f32)>) -> (Vec<(u8, u8, f32)>, Option<String>) {
        let Some(rank) = self.lockstep.as_ref().and_then(|l| l.local_rank) else { return (events, None) };
        let Some(pc) = self.coop.players.get(rank).copied().filter(|&p| self.alive(p)) else { return (events, None) };
        let mut line = self.lockstep.as_mut().map(|l| std::mem::take(&mut l.chat)).unwrap_or_default();
        let mut kept = Vec::with_capacity(events.len());
        let mut say = None;
        for (key, action, value) in events {
            let effect = if action == IST_TYPE {
                line.key_type(key)
            } else if line.open {
                line.key_event(key, action)
            } else if action == IST_PRESS && !crate::player_input::is_mouse_device(key) {
                let binding = self.lockstep.as_ref().and_then(|l| l.profiles.get(rank)?.as_ref()?.bindings.command(Self::key_name(i32::from(key))).map(str::to_string));
                match binding.as_deref().and_then(talk_binding) {
                    Some(team) if self.lockstep_chat_may_open(pc) => {
                        line.open(team);
                        ChatEffect::Taken
                    }
                    _ => ChatEffect::Pass,
                }
            } else {
                ChatEffect::Pass
            };
            match effect {
                ChatEffect::Pass => kept.push((key, action, value)),
                ChatEffect::Taken => {}
                // One line per command (a second Enter in the same step waits: never, it closed).
                ChatEffect::Send(s) => say = Some(s),
            }
        }
        if let Some(l) = self.lockstep.as_mut() {
            l.chat = line;
        }
        (kept, say)
    }

    /// Player `pc`'s sent line, at the step of their command on every machine: what
    /// Console.Typing.KeyEvent's Enter does with it (ConsoleCommand through the console, whose
    /// owner is the typer: UInteraction::execConsoleCommand, not a command from a controller).
    pub(crate) fn lockstep_chat_run(&mut self, pc: ObjId, line: &str) {
        // The typer's co-op name on their messages, for this command only (the game's player
        // name, in its saves and checkpoints, stays the profile's).
        let rank = self.coop.players.iter().position(|&p| p == pc);
        let names: Vec<Option<String>> = self.lockstep.as_ref().map(|l| l.profiles.iter().map(|p| p.as_ref().and_then(|p| p.name.clone())).collect()).unwrap_or_default();
        let shown = rank.and_then(|r| chat_names(&names).get(r).cloned().flatten());
        let pri = self.propr(pc, "PlayerReplicationInfo").as_object().filter(|&r| self.alive(r));
        let swap = match (pri, shown) {
            (Some(pri), Some(name)) => {
                let old = self.prop(pri, "PlayerName");
                self.set_prop(pri, "PlayerName", Value::Str(name));
                Some((pri, old))
            }
            _ => None,
        };
        let kept = std::mem::replace(&mut self.command_from_pc, false);
        let (taken, _) = self.console_command_out(pc, line);
        self.command_from_pc = kept;
        if let Some((pri, old)) = swap {
            if self.alive(pri) {
                self.set_prop(pri, "PlayerName", old);
            }
        }
        if !taken {
            self.log.push(format!("Log: lockstep chat: \"{line}\" not taken"));
        }
    }

    /// This machine's open line drawn on its own view by the console's code: the console is put
    /// in its Typing state with the line for the drawing only (`lockstep_chat_draw_end` puts
    /// back what the simulation had).
    pub(crate) fn lockstep_chat_draw_begin(&mut self) {
        let Some(typed) = self.lockstep.as_ref().filter(|l| l.chat.open && l.chat_draw.is_none()).map(|l| l.chat.typed.clone()) else { return };
        let Some(console) = self.lockstep_console() else { return };
        // The simulation's own console line (a player's typed console) keeps the screen.
        if self.propr(console, "bTyping").as_bool() {
            return;
        }
        let Some(class) = self.objects[console].as_ref().map(|o| o.class) else { return };
        let Some((typing_class, typing)) = self.find_state(class, "Typing").map(|(c, sd)| (c, sd.name.clone())) else { return };
        let (state, state_class) = self.objects[console].as_ref().map(|o| (o.state.clone(), o.state_class)).unwrap_or_default();
        let swap = DrawSwap { console, state, state_class, typed: self.prop(console, "TypedStr"), visible: self.prop(console, crate::interaction::B_VISIBLE) };
        if let Some(o) = self.objects[console].as_mut() {
            o.state = Some(typing);
            o.state_class = Some(typing_class);
        }
        self.set_prop(console, "TypedStr", Value::Str(typed));
        self.set_prop(console, crate::interaction::B_VISIBLE, Value::Bool(true));
        if let Some(l) = self.lockstep.as_mut() {
            l.chat_draw = Some(swap);
        }
    }

    /// See [`Vm::lockstep_chat_draw_begin`].
    pub(crate) fn lockstep_chat_draw_end(&mut self) {
        let Some(s) = self.lockstep.as_mut().and_then(|l| l.chat_draw.take()) else { return };
        if self.objects.get(s.console).is_none_or(|o| o.is_none()) {
            return;
        }
        if let Some(o) = self.objects[s.console].as_mut() {
            o.state = s.state;
            o.state_class = s.state_class;
        }
        self.set_prop(s.console, "TypedStr", s.typed);
        self.set_prop(s.console, crate::interaction::B_VISIBLE, s.visible);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(line: &mut ChatLine, s: &str) {
        for b in s.bytes() {
            line.key_type(b);
        }
    }

    #[test]
    fn talk_opens_skips_its_own_character_and_sends_on_enter() {
        let mut l = ChatLine::default();
        l.open(false);
        typed(&mut l, "thello");
        assert_eq!(l.typed, "Say hello");
        assert_eq!(l.key_event(8, IST_PRESS), ChatEffect::Taken);
        assert_eq!(l.typed, "Say hell");
        assert_eq!(l.key_event(13, IST_PRESS), ChatEffect::Send("Say hell".into()));
        assert!(!l.open);
        // Closed: nothing is the line's any more.
        assert_eq!(l.key_type(b'a'), ChatEffect::Pass);
        assert_eq!(l.key_event(87, IST_PRESS), ChatEffect::Pass);
    }

    #[test]
    fn team_talk_and_escape_as_the_console() {
        let mut l = ChatLine::default();
        l.open(true);
        assert_eq!(l.typed, "TeamSay ");
        // A release (a key held before the line opened) goes on to the game.
        assert_eq!(l.key_event(87, crate::player_input::IST_RELEASE), ChatEffect::Pass);
        // A press is the line's.
        assert_eq!(l.key_event(87, IST_PRESS), ChatEffect::Taken);
        // First Escape empties, the second closes.
        l.key_event(27, IST_PRESS);
        assert!(l.open && l.typed.is_empty());
        l.key_event(27, IST_PRESS);
        assert!(!l.open);
        // Enter on an empty line closes without sending.
        l.open(false);
        l.key_event(27, IST_PRESS);
        assert_eq!(l.key_event(13, IST_PRESS), ChatEffect::Taken);
        assert!(!l.open);
    }

    #[test]
    fn refused_characters_and_length() {
        let mut l = ChatLine::default();
        l.open(false);
        l.key_type(b't');
        typed(&mut l, "a~b`c\u{8}");
        assert_eq!(l.typed, "Say abc");
        typed(&mut l, &"x".repeat(400));
        assert_eq!(l.typed.len(), LINE_MAX);
    }

    #[test]
    fn history_up_and_down() {
        let mut l = ChatLine::default();
        for w in ["one", "two"] {
            l.open(false);
            l.key_type(b't');
            typed(&mut l, w);
            l.key_event(13, IST_PRESS);
        }
        l.open(false);
        l.key_event(38, IST_PRESS);
        assert_eq!(l.typed, "Say two");
        l.key_event(38, IST_PRESS);
        assert_eq!(l.typed, "Say one");
        l.key_event(40, IST_PRESS);
        assert_eq!(l.typed, "Say two");
    }

    #[test]
    fn chat_names_tell_the_players_apart() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(chat_names(&[s("Alan"), s("Kim")]), vec![s("Alan"), s("Kim")]);
        assert_eq!(chat_names(&[s("Alan"), s("alan ")]), vec![s("Alan"), s("alan 2")]);
        assert_eq!(chat_names(&[s("XIII"), None]), vec![s("XIII"), None]);
        assert_eq!(chat_names(&[s(" "), s("Kim")]), vec![None, s("Kim")]);
    }

    #[test]
    fn bindings_that_open_the_line() {
        assert_eq!(talk_binding("Talk"), Some(false));
        assert_eq!(talk_binding(" teamtalk "), Some(true));
        assert_eq!(talk_binding("Type"), None);
        assert_eq!(talk_binding("Fire"), None);
    }
}
