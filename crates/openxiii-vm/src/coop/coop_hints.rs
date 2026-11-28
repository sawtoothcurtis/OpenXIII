//! Co-op (two players or more): the levels' hints are seen by every player and shown to every
//! player.
//!
//! - `CWndFocusTrigger` (the white "sixth sense" frame around something to look at, with its
//!   sound): its WaitForBeingSeen.Timer only looks at MapInfo's player (the story player). Here it
//!   runs once with each living player as MapInfo's player, story player first, until one of them
//!   sets it off; when nobody does, the shortest wait of the tries is kept. The frame the script
//!   opened on that player's HUD is opened on every other local HUD with the same settings (a
//!   long frame, closed by its EventFinFocus event, is entered in each HUD's tFocusTrigger list
//!   as the script does), its sound once (the players share the speakers). When the frame's end
//!   event comes (WaitEndFocus.Trigger, coop_cine.rs), the entries of the other HUDs are removed
//!   first.
//! - `MangousteSSH1Controller` (SSH1): its frame on the Mongoose goes to one player's HUD: copied
//!   to the other HUDs (no sound, the script plays none).
//! - `CWndSFXTrigger` (the comic-book insets of almost every map): it gives each inset to the
//!   first HUD it finds. Each inset is copied to a window of every other local HUD (no second
//!   sound).
//!
//! Left as they are (a player's own view, not hints): HeadShotSFXTrigger, CWndExplosion,
//! CWndFalling, CamViewTrigger, the XIIISoloMessage messages. Solo and the faithful build are
//! unchanged: all of this only runs in co-op.

use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::NativeField as Field;
use crate::{Result, Vm};

const WINDOW_CLASS: &str = "XIII.CWndOnTrigger";
/// Slots of XIIIBaseHud.tCartoonFocus / tFocusTrigger.
const FOCUS_SLOTS: usize = 5;
/// CWndOnTrigger's settings, as CWndSFXTrigger.Timer fills them.
const PANEL_FIELDS: [&str; 26] = [
    "bSeeXIII",
    "bAnimatedInRealtime",
    "AnimationDuration",
    "Cam",
    "Tar",
    "vTarOffset",
    "CWndDuration",
    "vScrPosX",
    "vScrPosY",
    "CWndSizeX",
    "CWndSizeY",
    "CWndFOV",
    "CWndBorderColor",
    "FilterColor",
    "HighLight",
    "FilterTexture",
    "CWndAppearFX",
    "CWndSoundType",
    "OverlayMsg",
    "OnoTexture",
    "OnoPosX",
    "OnoPosY",
    "OnoSize",
    "OnoSizeY",
    "hCWndSound",
    "Instigator",
];

#[derive(Debug, Default, Clone)]
pub(crate) struct CoopHints {
    /// One of the calls below is running (its own calls are not watched again).
    busy: bool,
}

/// What a CWndFocusTrigger shows, read before its Timer runs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FocusShown {
    /// The trigger's name when the frame lasts until its EventFinFocus event ("" otherwise).
    pub trigger: String,
    pub focus_name: String,
    pub focus_class: String,
    pub at: [f32; 3],
    pub appear: i32,
    pub vignette: bool,
    pub duration: f32,
    pub zoom: bool,
    pub zoom_fov: f32,
    pub zoom_distance: f32,
    pub danger: bool,
    /// Path of the sound ("" for none).
    pub sound: String,
}

impl FocusShown {
    fn hud_args(&self, focus: ObjId) -> Vec<Value> {
        vec![
            Value::Object(Some(focus)),
            Value::Int(self.appear),
            Value::Bool(self.vignette),
            Value::Float(self.duration),
            Value::Bool(self.zoom),
            Value::Float(self.zoom_fov),
            Value::Float(self.zoom_distance),
            Value::Bool(self.danger),
        ]
    }
}

impl Vm {
    /// In Vm::call, co-op with two players or more: the calls of the hints. `None`: not one.
    pub(crate) fn coop_hints_call(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> Option<Result<(Value, Vec<Option<Value>>)>> {
        if self.coop.hints.busy || self.coop.players.len() < 2 {
            return None;
        }
        let name = self.functions[f].name.as_str();
        match name {
            name if name.eq_ignore_ascii_case("Timer") => {
                if self.is_a(this, "CWndFocusTrigger") && self.functions[f].state.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("WaitForBeingSeen")) {
                    return Some(self.coop_focus_timer(f, this, args.to_vec()));
                }
                let caller = self.this_stack.last().copied()?;
                if self.is_a(this, "CWndOnTrigger") && self.prop(this, "iPhase").as_int() == 0 && self.alive(caller) && self.is_a(caller, "CWndSFXTrigger") {
                    return Some(self.coop_panel_timer(f, this, args.to_vec()));
                }
                None
            }
            name if name.eq_ignore_ascii_case("AddHudCartoonFocus") => {
                let caller = self.this_stack.last().copied()?;
                if !self.alive(caller) || !self.is_a(caller, "MangousteSSH1Controller") || !self.is_a(this, "XIIIBaseHud") {
                    return None;
                }
                Some(self.coop_mongoose_focus(f, this, args.to_vec()))
            }
            name if name.eq_ignore_ascii_case("Trigger") && self.is_a(this, "CWndSFXCancelTrigger") => {
                self.coop.hints.busy = true;
                let r = self.call(f, this, args.to_vec());
                self.coop.hints.busy = false;
                Some(r)
            }
            _ => None,
        }
    }

    /// HUD of a local player.
    fn coop_hints_hud(&self, pc: ObjId) -> Option<ObjId> {
        if !self.alive(pc) || !self.coop_is_local(pc) {
            return None;
        }
        self.prop(pc, "MyHUD").as_object().filter(|&h| self.alive(h) && self.is_a(h, "XIIIBaseHud"))
    }

    /// The call as the script makes it, not watched again here.
    fn coop_hints_plain_call(&mut self, f: FuncId, this: ObjId, args: Vec<Option<Value>>) -> Result<(Value, Vec<Option<Value>>)> {
        let busy = std::mem::replace(&mut self.coop.hints.busy, true);
        let r = self.call(f, this, args);
        self.coop.hints.busy = busy;
        r
    }

    /// What CWndFocusTrigger `t` shows if it is set off now (its Timer's arguments).
    fn coop_focus_shown(&self, t: ObjId) -> Option<FocusShown> {
        let focus = self.prop(t, "Focus").as_object().filter(|&a| self.alive(a) && !self.native_prop(a, Field::DeleteMe).as_bool())?;
        let end = self.prop(t, "EventFinFocus").as_name();
        let long = !end.is_empty() && !end.eq_ignore_ascii_case("None");
        let kind = self.prop(t, "CWndFocusSoundType").as_int();
        let sounds = match self.prop(t, "hCWndFocus") {
            Value::Array(a) => a,
            v => vec![v],
        };
        let sound = sounds.get(if kind > 0 { (kind - 1) as usize } else { 0 }).map(|v| if let Value::Asset(p) = v { p.clone() } else { String::new() }).unwrap_or_default();
        Some(FocusShown {
            trigger: if long { self.objects[t].as_ref().map(|o| o.name.to_string()).unwrap_or_default() } else { String::new() },
            focus_name: self.objects[focus].as_ref().map(|o| o.name.to_string()).unwrap_or_default(),
            focus_class: self.objects[focus].as_ref().map(|o| self.classes[o.class].name.to_string()).unwrap_or_default(),
            at: self.native_value(focus, Field::Location).as_vector(),
            appear: self.prop(t, "CWndFocusAppearFX").as_int(),
            vignette: self.prop(t, "bUseVignette").as_bool(),
            duration: if long { -1.0 } else { self.prop(t, "FocusDuration").as_float() },
            zoom: self.prop(t, "zoomFocus").as_bool(),
            zoom_fov: self.prop(t, "zoomFOV").as_float(),
            zoom_distance: self.prop(t, "zoomDistance").as_float(),
            danger: self.prop(t, "bDanger").as_bool(),
            sound,
        })
    }

    /// CWndFocusTrigger.WaitForBeingSeen.Timer with each living player as MapInfo's player.
    fn coop_focus_timer(&mut self, f: FuncId, this: ObjId, args: Vec<Option<Value>>) -> Result<(Value, Vec<Option<Value>>)> {
        let Some(map_info) = self.coop_story_map_info().filter(|_| true) else { return self.coop_hints_plain_call(f, this, args) };
        let saved = (self.prop(map_info, "XIIIPawn"), self.prop(map_info, "XIIIController"));
        let shown = self.coop_focus_shown(this);
        let count = self.prop(this, "iTriggerCount").as_int();
        let mut result = None;
        let mut seer = None;
        let mut shortest: Option<f32> = None;
        self.coop.hints.busy = true;
        for i in 0..self.coop.players.len() {
            let Some(pawn) = self.coop_living_pawn(i) else { continue };
            self.set_prop(map_info, "XIIIPawn", Value::Object(Some(pawn)));
            self.set_prop(map_info, "XIIIController", Value::Object(Some(self.coop.players[i])));
            let r = self.call(f, this, args.clone());
            let failed = r.is_err();
            result = Some(r);
            if !self.alive(this) || self.native_prop(this, Field::DeleteMe).as_bool() {
                // Destroyed: set off (its last count), or its Focus is gone (nothing shown).
                if shown.is_some() {
                    seer = Some(i);
                }
                break;
            }
            let waiting = self.current_state(this).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("WaitForBeingSeen"));
            if !waiting || self.prop(this, "iTriggerCount").as_int() != count {
                seer = Some(i);
                break;
            }
            if failed {
                break;
            }
            let rate = self.native_prop(this, Field::TimerRate).as_float();
            if rate > 0.0 && shortest.is_none_or(|s| rate < s) {
                shortest = Some(rate);
            }
        }
        self.coop.hints.busy = false;
        self.set_prop(map_info, "XIIIPawn", saved.0);
        self.set_prop(map_info, "XIIIController", saved.1);
        let Some(result) = result else { return self.coop_hints_plain_call(f, this, args) };
        match (seer, shown) {
            (Some(rank), Some(shown)) => {
                self.log.push(format!("Log: Co-op: hint {} seen by player {}", shown.focus_name, rank + 1));
                {
                    self.coop_focus_show_others(this, rank, &shown);
                }
            }
            _ => {
                // Nobody: the shortest wait of the tries (the nearest player's).
                if let Some(rate) = shortest.filter(|_| self.alive(this)) {
                    self.set_native_prop(this, Field::TimerRate, Value::Float(rate));
                }
            }
        }
        result
    }

    /// The frame player `rank` set off, on every other player's HUD (and sound once if that
    /// player's HUD is on another machine).
    fn coop_focus_show_others(&mut self, trigger: ObjId, rank: usize, shown: &FocusShown) {
        let Some(focus) = self.actors_named(&shown.focus_name).filter(|&a| self.alive(a)) else { return };
        let seer = self.coop.players[rank];
        let seer_here = self.coop_hints_hud(seer).is_some();
        let mut first_local_pawn = None;
        self.coop.hints.busy = true;
        for (i, pc) in self.coop.players.clone().into_iter().enumerate() {
            if i == rank {
                continue;
            }
            let Some(hud) = self.coop_hints_hud(pc) else { continue };
            if first_local_pawn.is_none() {
                first_local_pawn = self.prop(pc, "Pawn").as_object().filter(|&p| self.alive(p));
            }
            if !shown.trigger.is_empty() {
                let n = self.prop(hud, "eNbHudCartoonFocus").as_int().max(0) as usize;
                if n >= FOCUS_SLOTS {
                    continue;
                }
                let mut list = match self.prop(hud, "tFocusTrigger") {
                    Value::Array(a) => a,
                    _ => continue,
                };
                if let Some(slot) = list.get_mut(n) {
                    *slot = Value::Object(Some(trigger));
                }
                self.set_prop(hud, "tFocusTrigger", Value::Array(list));
            }
            if let Err(e) = self.call_named(hud, "AddHudCartoonFocus", shown.hud_args(focus)) {
                self.warn(format!("co-op hint on {}: {e}", self.obj_label(hud)));
            }
        }
        self.coop.hints.busy = false;
        if !seer_here && !shown.sound.is_empty() {
            if let Some(p) = first_local_pawn {
                let mut a = [Value::Asset(shown.sound.clone())];
                let _ = crate::natives::call(self, "PlaySound", p, &mut a, &[true]);
            }
        }
    }

    /// First living actor named `name` (an object's own name, not its path).
    pub(crate) fn actors_named(&self, name: &str) -> Option<ObjId> {
        if name.is_empty() {
            return None;
        }
        self.actors.iter().copied().find(|&a| self.objects.get(a).and_then(|o| o.as_ref()).is_some_and(|o| o.name.eq_ignore_ascii_case(name)))
    }

    /// Before WaitEndFocus.Trigger of `trigger` runs with the HUD that holds it (coop_cine.rs):
    /// the other local HUDs drop their entry, the other machines theirs.
    pub(crate) fn coop_focus_close_others(&mut self, trigger: ObjId) {
        let holds = |vm: &Vm, hud: ObjId| match vm.propr(hud, "tFocusTrigger") {
            Value::Array(list) => list.iter().position(|v| v.as_object() == Some(trigger)),
            _ => None,
        };
        let mut holder_seen = false;
        for pc in self.coop.players.clone() {
            if !self.alive(pc) {
                continue;
            }
            let Some(hud) = self.prop(pc, "MyHUD").as_object().filter(|&h| self.alive(h)) else { continue };
            let Some(i) = holds(self, hud) else { continue };
            // The first one is the HUD the script closes (coop_focus_window_begin's holder).
            if !holder_seen {
                holder_seen = true;
                continue;
            }
            self.coop_hud_drop_focus(hud, i);
        }
    }

    /// WaitEndFocus.Trigger's removal of entry `i` of `hud`'s long frames.
    fn coop_hud_drop_focus(&mut self, hud: ObjId, i: usize) {
        let frames = self.prop(hud, "tCartoonFocus");
        if let Some(frame) = match &frames {
            Value::Array(a) => a.get(i).and_then(|v| v.as_object()),
            _ => None,
        }
        .filter(|&c| self.alive(c))
        {
            self.coop.hints.busy = true;
            let _ = self.call_named(frame, "RemoveMe", vec![]);
            self.coop.hints.busy = false;
        }
        for list in ["tFocusTrigger", "tCartoonFocus"] {
            if let Value::Array(mut a) = self.prop(hud, list) {
                if i < a.len() {
                    a.remove(i);
                    a.push(Value::Object(None));
                }
                self.set_prop(hud, list, Value::Array(a));
            }
        }
        let n = self.prop(hud, "eNbHudCartoonFocus").as_int();
        self.set_prop(hud, "eNbHudCartoonFocus", Value::Int((n - 1).max(0)));
    }

    /// MangousteSSH1Controller's frame on the Mongoose: on the other HUDs too.
    fn coop_mongoose_focus(&mut self, f: FuncId, this: ObjId, args: Vec<Option<Value>>) -> Result<(Value, Vec<Option<Value>>)> {
        let given: Vec<Value> = args.iter().map(|a| a.clone().unwrap_or(Value::Void)).collect();
        self.coop.hints.busy = true;
        let r = self.call(f, this, args);
        for pc in self.coop.players.clone() {
            let Some(hud) = self.coop_hints_hud(pc).filter(|&h| h != this) else { continue };
            let _ = self.call_named(hud, "AddHudCartoonFocus", given.clone());
        }
        self.coop.hints.busy = false;
        r
    }

    /// CWndOnTrigger.Timer (first phase) of a CWndSFXTrigger's inset: the same inset on a window
    /// of every other local HUD.
    fn coop_panel_timer(&mut self, f: FuncId, this: ObjId, args: Vec<Option<Value>>) -> Result<(Value, Vec<Option<Value>>)> {
        let settings: Vec<(&str, Value)> = PANEL_FIELDS.iter().map(|&k| (k, self.prop(this, k))).collect();
        let own_hud = self.prop(this, "MyHudForFX").as_object();
        self.coop.hints.busy = true;
        let r = self.call(f, this, args);
        {
            for pc in self.coop.players.clone() {
                let Some(hud) = self.coop_hints_hud(pc).filter(|&h| Some(h) != own_hud) else { continue };
                if let Err(e) = self.coop_panel_copy(pc, hud, &settings) {
                    self.warn(format!("co-op inset on {}: {e}", self.obj_label(hud)));
                }
            }
        }
        self.coop.hints.busy = false;
        r
    }

    /// A new inset window on `hud` (player controller `pc`) with `settings`, opened.
    fn coop_panel_copy(&mut self, pc: ObjId, hud: ObjId, settings: &[(&str, Value)]) -> std::result::Result<ObjId, String> {
        let class = self.class(WINDOW_CLASS).ok_or("no inset window class")?;
        let pawn = self.prop(pc, "Pawn").as_object().filter(|&p| self.alive(p));
        let owner = settings.iter().find(|s| s.0 == "Instigator").and_then(|s| s.1.as_object()).filter(|&a| self.alive(a)).or(pawn);
        let at = owner.map_or(Value::vector(0.0, 0.0, 0.0), |o| self.prop(o, "Location"));
        let w = self
            .spawn_actor(class, Value::Object(owner), None, at, Value::rotator(0, 0, 0), Value::Object(owner), true)
            .map_err(|e| e.to_string())?
            .ok_or("the inset window could not be made")?;
        for (k, v) in settings {
            if *k != "Instigator" {
                self.set_prop(w, k, v.clone());
            }
        }
        self.set_prop(w, "MyHudForFX", Value::Object(Some(hud)));
        // The players share the speakers: the first window's sound is enough.
        self.set_prop(w, "CWndSoundType", Value::Int(0));
        self.call_named(w, "Timer", Vec::new()).map_err(|e| e.to_string())?;
        Ok(w)
    }
}

