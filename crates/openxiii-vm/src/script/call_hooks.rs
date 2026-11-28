//! Script call hooks in dispatch order. A name selects candidates; the hooks themselves
//! check the receiver, function owner, state, arguments and current game settings.
use crate::types::{FuncId, HashMap};
use crate::value::{ObjId, Value};
use crate::{Result, Vm};

#[derive(Clone, Copy)]
enum Scope {
    General,
    Coop,
}

macro_rules! core_hooks {
    ($($group:ident { $($(#[$cfg:meta])* $hook:ident $scope:ident [$($name:literal),*];)* })*) => {
        #[derive(Clone, Copy)]
        enum Hook { $($($(#[$cfg])* $hook,)*)* Edition(crate::improved::hooks::Hook) }

        $(const $group: &[Registration] = &[
            $($(#[$cfg])* (Hook::$hook, Scope::$scope, &[$($name),*]),)*
        ];)*
    };
}

type Registration = (Hook, Scope, &'static [&'static str]);

// Empty name lists match every function, including functions added after loading.
core_hooks! {
    DIAGNOSTICS_AND_MODS {
        #[cfg(feature = "mods")]
        ModHit General ["TakeDamage"];
        #[cfg(feature = "mods")]
        ModSkin General ["ClientMessage"];
        #[cfg(feature = "mods")]
        ModEffect General ["ClientMessage"];
        #[cfg(feature = "mods")]
        ModGame General ["ClientMessage"];
        #[cfg(feature = "mods")]
        ModKey General ["ServerUpdateSkin"];
    }
    GAME {
        Versus General ["Paint", "Created", "InitViewportId"];
        LocalCoop General [];
        CoopEnemies Coop ["Died", "GeneratedPawn"];
        CoopAnswer Coop ["EndGame", "RestartPlayer", "SetPause", "OpenMenu", "FindPlayerController"];
        CoopTravel Coop ["ProcessServerTravel", "DoTravel"];
        CoopAi Coop ["NotifyKilled", "SeePlayer", "DamageAttitudeTo", "HearNoise"];
        CoopEndMessage Coop ["LocalizedMessage"];
        CoopEndHit Coop ["TakeDamage"];
        CoopMirror Coop ["LocalizedMessage"];
        CoopDialog Coop ["DrawDlg"];
        CoopShared Coop ["GiveObject", "AddBossBar", "AddChronoDisplay", "LocalizedMessage", "Tick", "Timer", "BeginState"];
        CoopScenery Coop ["ShowAndHide"];
        CoopPlaceEffect Coop ["UseCurrentColor", "TurnFilterOff", "InitDazzleAndShake"];
        CoopItem Coop ["Activate"];
        CoopGenerator Coop ["Timer"];
        CoopRules Coop ["Interpret", "Touch", "Trigger", "LockDoor", "PlayerTrigger"];
        CoopHook Coop ["ProcessTouch"];
        CoopSceneBind Coop ["Touch", "UnTouch", "Trigger"];
        CoopSceneLine Coop ["Interpret"];
        CoopHints Coop ["Timer", "AddHudCartoonFocus", "Trigger"];
        CoopEnd Coop ["EndGame"];
        CoopFocus Coop ["Trigger"];
        CoopStoryMachine Coop ["Trigger", "Touch", "BeginBeginFlash"];
        CoopFlash Coop ["SaveInventoryForFlash", "RestoreInventoryAfterFlash"];
        CoopActing Coop ["Touch", "UnTouch", "Trigger", "UnTrigger", "Bump", "UsedBy", "Used"];
        CoopImposed Coop [];
        CoopGift Coop ["GiveSomething"];
    }
    COMIC {
        ComicFrame General ["DrawWnd"];
    }
}

// Install each edition's hooks once in their original positions. Versus, public
// co-op state and the comic frame exist in both editions.
static ORDER: std::sync::LazyLock<Vec<Registration>> = std::sync::LazyLock::new(|| {
    use crate::improved::hooks;
    let mut order = DIAGNOSTICS_AND_MODS.to_vec();
    let extensions = |rules: &'static [(hooks::Hook, &'static [&'static str])]| {
        rules.iter().map(|&(hook, names)| (Hook::Edition(hook), Scope::General, names))
    };
    order.extend(extensions(hooks::BEFORE_VERSUS));
    order.extend_from_slice(GAME);
    order.extend(extensions(hooks::BEFORE_COMIC_FRAME));
    order.extend_from_slice(COMIC);
    order.extend(extensions(hooks::AFTER_COMIC_FRAME));
    assert!(order.len() <= u64::BITS as usize);
    order
});

pub(crate) type Arguments = Vec<Option<Value>>;
pub(crate) type CallResult = Result<(Value, Arguments)>;

#[derive(Clone, Copy, Default)]
struct Plan {
    general: u64,
    coop: u64,
}

impl Plan {
    fn for_name(name: &str, order: &[Registration]) -> Self {
        let mut plan = Self::default();
        for (index, &(_, scope, names)) in order.iter().enumerate() {
            if names.is_empty() || names.iter().any(|candidate| name.eq_ignore_ascii_case(candidate)) {
                let group = match scope {
                    Scope::General => &mut plan.general,
                    Scope::Coop => &mut plan.coop,
                };
                *group |= 1 << index;
            }
        }
        plan
    }
}

/// Prepared when a class or state function is loaded. Look up the current spelling so
/// dynamically added and renamed functions use the same rules as package functions.
pub(crate) struct CallHooks {
    order: &'static [Registration],
    by_name: HashMap<Box<str>, Plan>,
}

impl Default for CallHooks {
    fn default() -> Self {
        Self { order: &ORDER, by_name: HashMap::default() }
    }
}

impl CallHooks {
    fn plan(&mut self, name: &str) -> Plan {
        if let Some(&plan) = self.by_name.get(name) {
            return plan;
        }
        let plan = Plan::for_name(name, self.order);
        self.by_name.insert(name.into(), plan);
        plan
    }

    pub(crate) fn prepare(&mut self, name: &str) {
        self.plan(name);
    }
}

impl Vm {
    /// Calls a function and returns its value and final parameters. A hook may answer,
    /// delegate directly to the script body, or reenter the ordered hooks with a guard.
    pub fn call(&mut self, f: FuncId, this: ObjId, args: Vec<Option<Value>>) -> Result<(Value, Vec<Option<Value>>)> {
        let _math = self.session.math.enter();
        let plan = self.call_hooks.plan(&self.functions[f].name);
        let mut pending = plan.general;
        let mut acting = None;
        let order = self.call_hooks.order;
        let mut args = args;
        while pending != 0 {
            let index = pending.trailing_zeros() as usize;
            pending &= pending - 1;
            match order[index].0 {
                Hook::Edition(hook) => match hook.run(self, f, this, args) {
                    std::ops::ControlFlow::Break(result) => return result,
                    std::ops::ControlFlow::Continue(next) => args = next,
                },
                #[cfg(feature = "mods")]
                Hook::ModHit => {
                    if self.mods.script_events.watch_hits {
                        self.mod_note_call(f, this, &args);
                    }
                }
                #[cfg(feature = "mods")]
                Hook::ModSkin => {
                    if (self.mods.edits.is_some() || self.mods.behaviour.is_some()) && self.mod_net_client() && self.mod_net_client_message(f, &args) {
                        return Ok((Value::Void, vec![None; args.len()]));
                    }
                }
                #[cfg(feature = "mods")]
                Hook::ModEffect => {
                    if self.mods.script_sources.is_some() && self.mod_net_client() && self.mod_net_fx_message(f, &args) {
                        return Ok((Value::Void, vec![None; args.len()]));
                    }
                }
                #[cfg(feature = "mods")]
                Hook::ModGame => {
                    if self.mods.script_sources.is_some() && self.mod_net_client() && self.mod_script_guest_call(f, this, &args) {
                        return Ok((Value::Void, vec![None; args.len()]));
                    }
                }
                #[cfg(feature = "mods")]
                Hook::ModKey => {
                    if self.mods.scripts.is_some() && self.mod_guest_key_call(f, this, &args) {
                        return Ok((Value::Void, vec![None; args.len()]));
                    }
                }
                Hook::Versus => {
                    if self.coop.versus {
                        if let Some(r) = self.versus_call(f, this, &args) {
                            return r;
                        }
                    }
                }
                Hook::LocalCoop => {
                    if self.coop.on {
                        pending |= plan.coop;
                    }
                }
                Hook::CoopEnemies => {
                    if !self.coop.enemies.member.is_empty() || !self.coop.enemies.generators.is_empty() {
                        self.coop_enemies_before_call(f, this, &args);
                    }
                }
                Hook::CoopAnswer => {
                    if let Some(v) = self.coop_answers(f, this, &args) {
                        return Ok((v, vec![None; args.len()]));
                    }
                }
                Hook::CoopTravel => {
                    if let Some(kind) = self.coop_travel_rule_begin(f, this) {
                        let r = self.call_inner(f, this, args)?;
                        self.coop_travel_rule_end(kind, this);
                        return Ok(r);
                    }
                }
                Hook::CoopAi => {
                    if self.coop_before_call(f, this, &args) {
                        self.coop.ai_event = true;
                        let r = self.call(f, this, args);
                        self.coop.ai_event = false;
                        self.coop_after_ai_event(this);
                        return r;
                    }
                }
                Hook::CoopEndMessage => {
                    self.coop_end_note_message(f, this, &args);
                }
                Hook::CoopEndHit => {
                    self.coop_end_note_hit(f, this, &args);
                }
                Hook::CoopMirror => {
                    if self.coop_mirrors_message(f, this, &args) {
                        let copy = args.clone();
                        self.coop.mirroring = true;
                        let r = self.call(f, this, args);
                        self.coop_mirror_message(copy);
                        self.coop.mirroring = false;
                        return r;
                    }
                }
                Hook::CoopDialog => {
                    if let Some(layout) = self.coop_dialog_layout_before(f, this, &args) {
                        self.coop.dialog_layout_busy = true;
                        let r = self.call(f, this, args);
                        self.coop.dialog_layout_busy = false;
                        self.coop_dialog_layout_after(this, layout);
                        return r;
                    }
                }
                Hook::CoopShared => {
                    if let Some(shared) = self.coop_shared_before(f, this, &args) {
                        return self.coop_shared_call(f, this, args, shared);
                    }
                }
                Hook::CoopScenery => {
                    if let Some(list) = self.coop_scenery_begin(f, this) {
                        let r = self.call(f, this, args);
                        self.coop_scenery_end(this, list);
                        return r;
                    }
                }
                Hook::CoopPlaceEffect => {
                    if let Some(effect) = self.coop_place_effect_begin(f, this, &args) {
                        let r = self.call(f, this, args);
                        self.coop_place_effect_end(this, effect);
                        return r;
                    }
                }
                Hook::CoopItem => {
                    self.coop_item_setup(f, this);
                }
                Hook::CoopGenerator => {
                    if let Some(saved) = self.coop_generator_begin(f, this) {
                        let r = self.call(f, this, args);
                        self.coop_generator_end(this, saved);
                        return r;
                    }
                }
                Hook::CoopRules => {
                    if let Some(after) = self.coop_rules_before(f, this, &args) {
                        return self.coop_rules_call(f, this, args, after);
                    }
                }
                Hook::CoopHook => {
                    if self.coop_hook_passes(f, this, &args) {
                        return Ok((Value::Void, vec![None; args.len()]));
                    }
                }
                Hook::CoopSceneBind => {
                    self.coop_scene_bind(f, this, &args);
                }
                Hook::CoopSceneLine => {
                    if let Some(owner) = self.coop_scene_line_begin(f, this) {
                        let r = self.call(f, this, args);
                        self.coop_flash_owner_end(owner);
                        return r;
                    }
                }
                Hook::CoopHints => {
                    if let Some(r) = self.coop_hints_call(f, this, &args) {
                        return r;
                    }
                }
                Hook::CoopEnd => {
                    if let Some(r) = self.coop_end_call(f, this, &args) {
                        return r;
                    }
                }
                Hook::CoopFocus => {
                    match self.coop_focus_window_begin(f, this) {
                        Some(Some(owner)) => {
                            self.coop_focus_close_others(this);
                            // With that player only: not the one who caused the event (coop_acting_begin).
                            let acting = std::mem::replace(&mut self.coop.acting, true);
                            let r = self.call(f, this, args);
                            self.coop.acting = acting;
                            self.coop.cine_rules.focus_busy = false;
                            self.coop_flash_owner_end(owner);
                            return r;
                        }
                        Some(None) => {
                            self.coop_focus_close_others(this);
                            return Ok((Value::Void, vec![None; args.len()]));
                        }
                        None => {}
                    }
                }
                Hook::CoopStoryMachine => {
                    if let Some(owner) = self.coop_story_machine_begin(f, this, &args) {
                        let r = self.call(f, this, args);
                        self.coop_flash_owner_end(owner);
                        return r;
                    }
                }
                Hook::CoopFlash => {
                    if let Some(owner) = self.coop_flash_owner_begin(f, this, &args) {
                        let r = self.call(f, this, args);
                        self.coop_flash_owner_end(owner);
                        return r;
                    }
                }
                Hook::CoopActing => {
                    acting = self.coop_acting_begin(f, &args);
                }
                Hook::CoopImposed => {
                    let acting = acting.take();
                    // What a MapInfo function imposes on its player is given to every player.
                    if let Some(before) = self.coop_imposed_begin(this) {
                        let r = self.call(f, this, args);
                        self.coop_imposed_end(before);
                        if let Some(saved) = acting {
                            self.coop_acting_end(saved);
                        }
                        return r;
                    }
                    if let Some(saved) = acting {
                        let r = self.call(f, this, args);
                        self.coop_acting_end(saved);
                        return r;
                    }
                }
                Hook::CoopGift => {
                    if self.coop_shares_gift(f) {
                        let gift = args.clone();
                        let r = self.call_inner(f, this, args)?;
                        self.coop_share_gift(f, this, gift);
                        return Ok(r);
                    }
                }
                Hook::ComicFrame => {
                    if let Some((w, fields_list)) = self.cases_bd_frame_43(f, this, &args) {
                        let r = self.call(f, this, args);
                        self.cases_bd_frame_43_end(w, fields_list);
                        return r;
                    }
                }
            }
        }
        // A singular function has no effect while the object is already running one.
        if self.functions[f].flags & openxiii_script::objects::func::SINGULAR == 0 {
            return self.call_inner(f, this, args);
        }
        if !self.in_singular.insert(this) {
            let ret = self.functions[f].ret.clone().map(|t| self.zero(&t, 1)).unwrap_or(Value::Void);
            let outs = self.functions[f].params.iter().enumerate().map(|(i, p)| if p.out { Some(args.get(i).cloned().flatten().unwrap_or(Value::Void)) } else { None }).collect();
            return Ok((ret, outs));
        }
        let r = self.call_inner(f, this, args);
        self.in_singular.remove(&this);
        r
    }
}

#[cfg(test)]
#[path = "../../tests/unit/script/call_hooks_test.rs"]
mod tests;
