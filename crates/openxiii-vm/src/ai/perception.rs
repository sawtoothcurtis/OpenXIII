//! Controller perception, reimplemented from Engine.dll: sight (AController::LineOfSightTo, SeePawn,
//! ShowSelf, CheckEnemyVisible, the SightCounter part of AController::Tick and
//! APlayerController::Tick), hearing (AActor::CheckNoiseHearing, AController::CanHear,
//! execMakeNoise) and PlayerCanSeeMe. Addresses at the top of each function; floating-point
//! operations follow the machine code's order.

use crate::ai_move::{fmul_d, rotator_vector};
use crate::physics::*;
use crate::value::{ObjId, Value};
use crate::Vm;

/// Sight traces: TRACE_World, TRACE_StopAtFirstHit and the exclusion of bCanSeeThrough actors.
const TRACE_SIGHT: u32 = 0x2286;

/// FVector::Normalize: (X² + Y²) + Z², nothing below 1e-8.
fn normalize(v: V) -> V {
    let sq = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
    if (sq as f64) < 1e-8 {
        return v;
    }
    let s = 1.0 / sq.sqrt();
    [s * v[0], s * v[1], s * v[2]]
}

/// Squared distance in the order (X² + Y²) + Z².
fn dist_sq_xyz(d: V) -> f32 {
    (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]
}

/// What the sight trace hit: nothing, the actor looked at, or something else.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Contact {
    Nothing,
    Other,
    SomethingElse,
}

/// Inputs of AController::LineOfSightTo read from the controller, its view target and the other actor.
#[derive(Clone, Debug)]
pub(crate) struct InputsLdv {
    /// Location of the view target (GetViewTarget).
    pub view_value: V,
    /// BaseEyeHeight of the view target, counted only if it is the controller's pawn.
    pub view_eye: f32,
    pub view_is_pawn: bool,
    pub other_one: V,
    pub other_eye: f32,
    pub other_height: f32,
    pub is_enemy: bool,
    /// bIsPlayer of the controller.
    pub player_value: bool,
    pub other_is_pawn: bool,
    /// bLOSflag of the controller.
    pub los_flag: bool,
    pub use_los_flag: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OutputLdv {
    pub sees: bool,
    /// (LastSeeingPos, LastSeenPos) written when the enemy is seen.
    pub last_view: Option<(V, V)>,
}

/// AController::LineOfSightTo without the world: `trace(end, start)` does
/// SingleLineCheck (TRACE_SIGHT). Nothing between the eyes and the actor (or its head for the enemy, or at
/// 80 % of its height beyond 2000 units for the others, up to 8000 for a player who
/// looks at a pawn).
pub(crate) fn line_view(e: &InputsLdv, trace: &mut dyn FnMut(V, V) -> Contact) -> OutputLdv {
    let unseen = OutputLdv { sees: false, last_view: None };
    let yes = OutputLdv { sees: true, last_view: None };
    let mut view = e.view_value;
    if e.view_is_pawn {
        view[2] += e.view_eye;
    }
    let ol = e.other_one;
    if e.is_enemy {
        let mut blocker = trace(ol, view);
        if blocker == Contact::SomethingElse {
            let eyes = [ol[0], ol[1], e.other_eye + ol[2]];
            blocker = trace(eyes, view);
            if blocker == Contact::SomethingElse {
                return unseen;
            }
        }
        return OutputLdv { sees: true, last_view: Some((e.view_value, ol)) };
    }
    if trace(ol, view) != Contact::SomethingElse {
        return yes;
    }
    let dist_sq = dist_sq_xyz(sub(ol, e.view_value));
    if dist_sq > 6.4e7 {
        return unseen;
    }
    if !(e.player_value && e.other_is_pawn) && dist_sq > 4e6 {
        return unseen;
    }
    if e.use_los_flag && e.los_flag {
        return unseen;
    }
    let top = [ol[0], ol[1], fmul_d(e.other_height, 0.8) + ol[2]];
    if trace(top, view) != Contact::SomethingElse { yes } else { unseen }
}

pub(crate) trait WorldView {
    fn rand(&mut self) -> f32;
    fn ldv(&mut self, use_los_flag: bool, los_flag: bool) -> bool;
    fn note(&mut self, _step: StepView) {}
}

#[allow(dead_code)]
pub(crate) enum StepView {
    Far { dist: f32, r: f32 },
    Zone { dist: f32, r: f32, pass: bool },
    Cone { dist: f32, stimulus: f32, peripheral: f32, look: V },
}

/// Inputs of AController::SeePawn read from the controller, its pawn and the other pawn.
#[derive(Clone, Debug)]
pub(crate) struct InputsView {
    pub is_enemy: bool,
    pub los_flag: bool,
    /// Location of the controller's pawn.
    pub pawn_value: V,
    pub other_one: V,
    /// Visibility of the other pawn.
    pub visibility: u8,
    /// SightRadius and PeripheralVision of the controller's pawn.
    pub sight_radius: f32,
    pub peripheral: f32,
    /// Rotation of the controller.
    pub rotation: [i32; 3],
    /// Both pawns in the same zone (Region.Zone).
    pub same_zone_flag: bool,
    pub may_skip: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OutputView {
    pub sees: bool,
    /// bLOSflag after the call.
    pub los_flag: bool,
    /// Last Stimulus written, if there is one.
    pub stimulus: Option<f32>,
    pub ldv_called: bool,
    /// Range SightRadius × Visibility / 128 (log).
    pub max: f32,
}

/// AController::SeePawn without the world: range SightRadius × Visibility / 128, field
/// of view PeripheralVision around the controller's rotation, XIII's random checks
/// when `may_skip` (far, other zone, low view), then LineOfSightTo.
pub(crate) fn sees_pawn(e: &InputsView, m: &mut impl WorldView) -> OutputView {
    let mut s = OutputView { sees: false, los_flag: e.los_flag, stimulus: None, ldv_called: false, max: 0.0 };
    if e.is_enemy {
        s.ldv_called = true;
        s.sees = m.ldv(false, s.los_flag);
        return s;
    }
    s.los_flag = !e.los_flag;
    let delta = sub(e.other_one, e.pawn_value);
    let dist_sq = dist_sq_xyz(delta);
    let mut max = e.visibility as i32 as f32 * 0.0078125;
    if 1.0 <= max {
        max = 1.0;
    }
    let max = max * e.sight_radius;
    s.max = max;
    let max_sq = max * max;
    if dist_sq > max_sq {
        return s;
    }
    if e.may_skip {
        if max_sq * 0.25 < dist_sq {
            let r = m.rand();
            m.note(StepView::Far { dist: dist_sq.sqrt(), r });
            if (r as f64) < 0.5 {
                return s;
            }
        }
        if !e.same_zone_flag {
            let r = m.rand();
            m.note(StepView::Zone { dist: dist_sq.sqrt(), r, pass: max_sq * 0.04 >= r * dist_sq });
            if max_sq * 0.04 < r * dist_sq {
                return s;
            }
        }
    }
    let mut sight = safe_normal(delta);
    let look = rotator_vector(e.rotation);
    let stimulus = (look[1] * sight[1] + look[0] * sight[0]) + look[2] * sight[2];
    s.stimulus = Some(stimulus);
    m.note(StepView::Cone { dist: dist_sq.sqrt(), stimulus, peripheral: e.peripheral, look });
    if stimulus < e.peripheral {
        return s;
    }
    if e.may_skip {
        let r = m.rand();
        if max_sq * 0.01 < r * dist_sq {
            // Low view: height counts double.
            sight[2] += sight[2];
            sight = normalize(sight);
            if (look[1] * sight[1] + look[0] * sight[0]) + look[2] * sight[2] < e.peripheral {
                return s;
            }
            let h = (e.other_one[2] - e.pawn_value[2]).abs();
            let r = m.rand();
            if h / dist_sq * h > r {
                return s;
            }
        }
    }
    s.stimulus = Some(1.0);
    s.ldv_called = true;
    s.sees = m.ldv(e.may_skip, s.los_flag);
    s
}

/// SeePawn in the VM: the VM's draws, the VM's LineOfSightTo.
struct WorldVm<'a> {
    vm: &'a mut Vm,
    c: ObjId,
    other: ObjId,
}

impl WorldView for WorldVm<'_> {
    #[track_caller]
    fn rand(&mut self) -> f32 {
        self.vm.rand()
    }
    fn ldv(&mut self, use_los_flag: bool, los_flag: bool) -> bool {
        self.vm.set_b(self.c, "bLOSflag", los_flag);
        self.vm.line_of_sight_to(self.c, Some(self.other), use_los_flag)
    }
}

/// Result of a SingleLineCheck as seen by CanHear: a hit actor and Hit.Location (left as
/// is by a miss, like `Vm::single_line_check`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Impact {
    pub key_col: bool,
    pub location: V,
}

impl Impact {
    /// FCheckResult(1.0): zero Location.
    fn fresh_value() -> Self {
        Impact { key_col: false, location: [0.0; 3] }
    }
}

/// What CanHear asks of the world: SingleLineCheck, the map's navigation points (in
/// NavigationPointList order, with Location and bPropagatesSound) and UModel::FastLineCheck.
pub(crate) trait WorldHearing {
    fn trace(&mut self, end: V, start: V, flags: u32, hit: &mut Impact);
    fn points(&mut self) -> Vec<(ObjId, V, bool)>;
    fn free_line(&mut self, end: V, start: V) -> bool;
}

/// Inputs of AController::CanHear read from the controller, its pawn and the actor making the noise.
#[derive(Clone, Debug)]
pub(crate) struct InputsHearing {
    /// The controller has a pawn; the other actor's Instigator has a controller.
    pub to_pawn: bool,
    pub instigator: bool,
    pub pawn_value: V,
    pub noise_value: V,
    pub loudness: f32,
    pub hearing_threshold: f32,
    pub alertness: f32,
    pub eye: f32,
    pub same_zone: bool,
    pub adjacent: bool,
    pub los_hearing: bool,
    pub muffled: bool,
    pub around_corner: bool,
    /// Region.Zone of the pawn and of the other actor identical.
    pub same_zone_flag: bool,
    /// ZoneConnectivity of the pawn's zone and ZoneNumber of the other actor.
    pub connectivity: u64,
    pub zone_other: u8,
    /// Location of the other actor.
    pub other_one: V,
}

pub(crate) struct OutputHearing {
    pub hears: bool,
    pub perceived: f32,
    pub dist_sq: f32,
    pub los_blocked: Option<bool>,
    /// Reached the bPropagatesSound points (after the line of sight and the thin wall).
    pub coin: bool,
}

/// AController::CanHear without the world: range HearingThreshold² × Loudness ×
/// (Alertness + 1), same zone or neighbouring zone, otherwise line of sight (bLOSHearing), sound muffled by a
/// thin wall (bMuffledHearing) or routed around through the bPropagatesSound points (bAroundCornerHearing).
pub(crate) fn hears(e: &InputsHearing, m: &mut dyn WorldHearing) -> OutputHearing {
    let mut s = OutputHearing { hears: false, perceived: 0.0, dist_sq: 0.0, los_blocked: None, coin: false };
    if !e.instigator || !e.to_pawn {
        return s;
    }
    let pl = e.pawn_value;
    let mut dist_sq = dist_sq_xyz(sub(pl, e.noise_value));
    let ht = e.hearing_threshold;
    let mut alert = e.alertness + 1.0;
    if 0.0 >= alert {
        alert = 0.0;
    }
    let mut perceived = ((ht * ht) * e.loudness) * alert;
    s.perceived = perceived;
    s.dist_sq = dist_sq;
    if perceived < dist_sq {
        return s;
    }
    if e.same_zone || e.adjacent {
        if e.same_zone_flag {
            s.hears = true;
            return s;
        }
        if e.adjacent {
            let conn = e.connectivity;
            // 1 << ZoneNumber on 32 bits, sign-extended (machine code).
            let lo = 1u32 << (e.zone_other as u32 & 31);
            let hi = if lo & 0x8000_0000 != 0 { u32::MAX } else { 0 };
            if (conn as u32 & lo) | ((conn >> 32) as u32 & hi) != 0 {
                s.hears = true;
                return s;
            }
        }
    }
    if !e.los_hearing {
        return s;
    }
    let view = [pl[0], pl[1], e.eye + pl[2]];
    let mut hit = Impact::fresh_value();
    m.trace(e.noise_value, view, 0x286, &mut hit);
    s.los_blocked = Some(hit.key_col);
    if !hit.key_col {
        s.hears = true;
        return s;
    }
    if e.muffled {
        dist_sq *= 4.0;
        s.dist_sq = dist_sq;
        if dist_sq < perceived {
            // ULevel::SingleLineCheck always copies into Hit its local variable, which a
            // miss does not fill (only Time and Actor): it keeps the result of the previous call
            // at the same stack depth, here the sight trace then the first wall trace.
            let mut h = hit;
            m.trace(e.noise_value, view, TRACE_LEVEL, &mut h);
            let l1 = h.location;
            m.trace(view, e.noise_value, TRACE_LEVEL, &mut h);
            let d = sub(l1, h.location);
            let wall = dist_sq_xyz(d);
            if wall * wall + dist_sq < perceived {
                s.hears = true;
                return s;
            }
        }
    }
    s.coin = true;
    if e.around_corner {
        perceived *= 0.125;
        let ol = e.other_one;
        // FSortedPathList::addPath of the bPropagatesSound points close to both.
        let mut points = crate::navigation::SortedPathList::new();
        let mut places = std::collections::HashMap::new();
        for (n, nl, propagates) in m.points() {
            if !propagates {
                continue;
            }
            let a = dist_sq_xyz(sub(nl, pl));
            let b = dist_sq_xyz(sub(nl, ol));
            if a < perceived && b < perceived {
                points.add_path(n, b + a);
                places.insert(n, nl);
            }
        }
        for n in points.paths() {
            let nl = places[&n];
            if m.free_line(nl, e.noise_value) && m.free_line(nl, view) {
                s.hears = true;
                return s;
            }
        }
    }
    s
}

/// CanHear in the VM.
struct WorldHearingVm<'a> {
    vm: &'a Vm,
    c: ObjId,
}

impl WorldHearing for WorldHearingVm<'_> {
    fn trace(&mut self, end: V, start: V, flags: u32, hit: &mut Impact) {
        let mut h = CheckResult::new(1.0);
        h.location = hit.location;
        self.vm.single_line_check(&mut h, Some(self.c), end, start, flags, [0.0; 3]);
        *hit = Impact { key_col: h.actor.is_some(), location: h.location };
    }
    fn points(&mut self) -> Vec<(ObjId, V, bool)> {
        self.vm.navigation_points().into_iter().map(|n| (n, self.vm.native_prop(n, crate::NativeField::Location).as_vector(), self.vm.pb(n, "bPropagatesSound"))).collect()
    }
    fn free_line(&mut self, end: V, start: V) -> bool {
        self.vm.fast_line_check(end, start)
    }
}

impl Vm {

    fn sight_trace(&self, c: ObjId, end: V, start: V) -> Option<ObjId> {
        let mut hit = CheckResult::new(1.0);
        self.single_line_check(&mut hit, Some(c), end, start, TRACE_SIGHT, [0.0; 3]);
        hit.actor
    }

    /// AController::GetViewTarget (vt 0x140): the pawn (or the controller); ViewTarget for a
    /// PlayerController.
    fn view_target(&self, c: ObjId) -> Option<ObjId> {
        if self.is_a(c, "PlayerController") {
            return self.po(c, "ViewTarget");
        }
        Some(self.native_object(c, crate::NativeField::Pawn).unwrap_or(c))
    }

    /// AController::LineOfSightTo: nothing of the map (except bCanSeeThrough) between the
    /// eyes and the actor (see `line_view`).
    pub(crate) fn line_of_sight_to(&mut self, c: ObjId, other: Option<ObjId>, use_los_flag: bool) -> bool {
        let Some(other) = other.filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) else { return false };
        let Some(vt) = self.view_target(c) else { return false };
        let view_is_pawn = Some(vt) == self.native_object(c, crate::NativeField::Pawn);
        let e = InputsLdv {
            view_value: self.native_prop(vt, crate::NativeField::Location).as_vector(),
            view_eye: if view_is_pawn { self.pf(vt, "BaseEyeHeight") } else { 0.0 },
            view_is_pawn,
            other_one: self.native_prop(other, crate::NativeField::Location).as_vector(),
            other_eye: self.pf(other, "BaseEyeHeight"),
            other_height: self.native_prop(other, crate::NativeField::CollisionHeight).as_float(),
            is_enemy: Some(other) == self.po(c, "Enemy"),
            player_value: self.pb(c, "bIsPlayer"),
            other_is_pawn: self.is_a(other, "Pawn"),
            los_flag: self.pb(c, "bLOSflag"),
            use_los_flag,
        };
        let s = line_view(&e, &mut |end, start| match self.sight_trace(c, end, start) {
            None => Contact::Nothing,
            Some(a) if a == other => Contact::Other,
            Some(_) => Contact::SomethingElse,
        });
        if let Some((seeing, seen)) = s.last_view {
            self.set_v(c, "LastSeeingPos", seeing);
            self.set_v(c, "LastSeenPos", seen);
        }
        s.sees
    }

    /// AController::SeePawn: see `sees_pawn`.
    pub(crate) fn see_pawn(&mut self, c: ObjId, other: Option<ObjId>, may_skip: bool) -> bool {
        let (Some(other), Some(p)) = (other.filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())), self.native_object(c, crate::NativeField::Pawn)) else { return false };
        let zone = |vm: &Vm, a: ObjId| match vm.prop(a, "Region") {
            Value::Struct(_, f) => f.first().and_then(|z| z.as_object()),
            _ => None,
        };
        let e = InputsView {
            is_enemy: Some(other) == self.po(c, "Enemy"),
            los_flag: self.pb(c, "bLOSflag"),
            pawn_value: self.native_prop(p, crate::NativeField::Location).as_vector(),
            other_one: self.native_prop(other, crate::NativeField::Location).as_vector(),
            visibility: self.propr(other, "Visibility").as_byte(),
            sight_radius: self.pf(p, "SightRadius"),
            peripheral: self.pf(p, "PeripheralVision"),
            rotation: self.propr(c, "Rotation").as_rotator(),
            same_zone_flag: zone(self, p) == zone(self, other),
            may_skip,
        };
        let s = sees_pawn(&e, &mut WorldVm { vm: self, c, other });
        self.set_b(c, "bLOSflag", s.los_flag);
        if let Some(st) = s.stimulus {
            self.set_f(c, "Stimulus", st);
        }
        s.sees
    }

    /// AController::ShowSelf: the controllers whose sight check is due (SightCounter < 0)
    /// and that see the pawn receive SeePlayer (a player's pawn) or SeeMonster.
    pub(crate) fn show_self(&mut self, c: ObjId) {
        self.show_self_with(c, |vm, o, event| vm.is_probing(o, event), |vm, o, p| vm.see_pawn(o, p, true));
    }

    /// `probing`: UObject::IsProbing (Core); `see`: AController::SeePawn (bMaySkipChecks = 1); stubbed in the tests.
    pub(crate) fn show_self_with(&mut self, c: ObjId, mut probing: impl FnMut(&Vm, ObjId, &str) -> bool, mut see: impl FnMut(&mut Vm, ObjId, Option<ObjId>) -> bool) {
        let Some(p) = self.native_object(c, crate::NativeField::Pawn) else { return };
        let is_player = self.pb(c, "bIsPlayer");
        let mut other = self.level.and_then(|l| self.po(l, "ControllerList"));
        let mut guard = 0;
        while let Some(o) = other {
            guard += 1;
            if guard > 10_000 {
                break;
            }
            if o != c && self.native_prop(o, crate::NativeField::SightCounter).as_float() < 0.0 {
                // AController::eventSeePlayer / eventSeeMonster, from ShowSelf.
                let event = if is_player { "SeePlayer" } else { "SeeMonster" };
                let allowed = is_player || !self.pb(o, "bDisableEventSeeMonster");
                if allowed && probing(self, o, event) && see(self, o, Some(p)) {
                    self.event_pub(o, event, vec![Value::Object(Some(p))]);
                }
            }
            other = self.native_object(o, crate::NativeField::NextController);
        }
    }

    /// AController::CheckEnemyVisible: LastSeenTime, or EnemyNotVisible.
    fn check_enemy_visible(&mut self, c: ObjId) {
        self.check_enemy_visible_with(c, |vm, c, enemy| vm.line_of_sight_to(c, enemy, false));
    }

    /// `los`: AController::LineOfSightTo of the enemy, bUseLOSFlag at 0 (stubbed in the tests).
    pub(crate) fn check_enemy_visible_with(&mut self, c: ObjId, los: impl FnOnce(&mut Vm, ObjId, Option<ObjId>) -> bool) {
        let Some(enemy) = self.po(c, "Enemy") else { return };
        if los(self, c, Some(enemy)) {
            let t = self.level_time as f32;
            self.set_f(c, "LastSeenTime", t);
        } else {
            // AController::eventEnemyNotVisible.
            self.event_pub(c, "EnemyNotVisible", vec![]);
        }
    }

    /// AController::Tick, perception: sight of the enemy every 0.1 s (0.2 s without
    /// EnemyNotVisible), then the pawn shows itself to the others.
    pub(crate) fn controller_perception(&mut self, c: ObjId, dt: f32) {
        let mut sight = self.native_prop(c, crate::NativeField::SightCounter).as_float();
        if sight < 0.0 {
            if self.is_probing(c, "EnemyNotVisible") {
                self.check_enemy_visible(c);
                if self.objects[c].is_none() {
                    return;
                }
                sight = self.native_prop(c, crate::NativeField::SightCounter).as_float() + 0.1;
            } else {
                sight += 0.2;
            }
            self.set_native_prop(c, crate::NativeField::SightCounter, Value::Float(sight));
        }
        let sight = self.native_prop(c, crate::NativeField::SightCounter).as_float() - dt;
        self.set_native_prop(c, crate::NativeField::SightCounter, Value::Float(sight));
        if let Some(p) = self.native_object(c, crate::NativeField::Pawn) {
            if !self.native_prop(p, crate::NativeField::Hidden).as_bool() && !self.pb(p, "bAmbientCreature") {
                self.show_self(c);
            }
        }
    }

    /// APlayerController::Tick, the player's perception.
    pub(crate) fn player_perception(&mut self, pc: ObjId, dt: f32) {
        let mut sight = self.native_prop(pc, crate::NativeField::SightCounter).as_float();
        if sight < 0.0 {
            sight += 0.2;
        }
        self.set_native_prop(pc, crate::NativeField::SightCounter, Value::Float(sight - dt));
        if self.native_object(pc, crate::NativeField::Pawn).is_some_and(|p| !self.native_prop(p, crate::NativeField::Hidden).as_bool()) {
            self.show_self(pc);
        }
    }

    /// APawn::IsPlayer.
    fn pawn_is_player(&self, p: ObjId) -> bool {
        self.native_object(p, crate::NativeField::Controller).is_some_and(|c| self.pb(c, "bIsPlayer"))
    }

    /// AController::CanHear: see `hears`.
    fn can_hear(&mut self, c: ObjId, noise: V, loudness: f32, other: ObjId) -> bool {
        let instigator = self.po(other, "Instigator").and_then(|i| self.native_object(i, crate::NativeField::Controller)).is_some();
        let pawn_value = self.native_object(c, crate::NativeField::Pawn);
        let region = |vm: &Vm, a: ObjId| match vm.prop(a, "Region") {
            Value::Struct(_, f) => (f.first().and_then(|z| z.as_object()), f.get(2).map_or(0, |z| z.as_int())),
            _ => (None, 0),
        };
        let Some(p) = pawn_value.filter(|_| instigator) else { return false };
        let ((pz, pn), (oz, on)) = (region(self, p), region(self, other));
        let e = InputsHearing {
            to_pawn: true,
            instigator,
            pawn_value: self.native_prop(p, crate::NativeField::Location).as_vector(),
            noise_value: noise,
            loudness,
            hearing_threshold: self.pf(p, "HearingThreshold"),
            alertness: self.pf(p, "Alertness"),
            eye: self.pf(p, "BaseEyeHeight"),
            same_zone: self.pb(p, "bSameZoneHearing"),
            adjacent: self.pb(p, "bAdjacentZoneHearing"),
            los_hearing: self.pb(p, "bLOSHearing"),
            muffled: self.pb(p, "bMuffledHearing"),
            around_corner: self.pb(p, "bAroundCornerHearing"),
            same_zone_flag: pz == oz,
            connectivity: self.level_bsp.as_ref().and_then(|b| b.zone_connectivity.get(pn as usize)).copied().unwrap_or(0) as u64,
            zone_other: on as u8,
            other_one: self.native_prop(other, crate::NativeField::Location).as_vector(),
        };
        let s = hears(&e, &mut WorldHearingVm { vm: self, c });
        s.hears
    }

    /// UModel::FastLineCheck of the map's BSP: true if the segment only crosses
    /// empty space.
    fn fast_line_check(&self, end: V, start: V) -> bool {
        let Some(bsp) = self.level_bsp.as_ref() else { return true };
        bsp.fast_line_check(end, start)
    }

    /// AActor::execMakeNoise then AActor::CheckNoiseHearing: one noise
    /// per 0.2 s and per place (the Instigator's noise1 / noise2 slots, unless the noise is louder),
    /// HearNoise to the controllers that hear it; the noise of an NPC without a player enemy is only
    /// heard by controllers with the same Tag and by players.
    pub(crate) fn make_noise(&mut self, a: ObjId, loudness: f32) {
        let Some(inst) = self.po(a, "Instigator") else { return };
        if self.native_object(inst, crate::NativeField::Controller).is_none() {
            return;
        }
        let now = self.level_time;
        let loc = self.native_prop(a, crate::NativeField::Location).as_vector();
        let mut c = Slots {
            time1: self.pf(inst, "noise1time"),
            spot1: self.pv(inst, "noise1spot"),
            loud1: self.pf(inst, "noise1loudness"),
            time2: self.pf(inst, "noise2time"),
            spot2: self.pv(inst, "noise2spot"),
            loud2: self.pf(inst, "noise2loudness"),
        };
        let chosen_slot = slots_noise(now, loc, loudness, &mut c);
        self.set_f(inst, "noise1time", c.time1);
        self.set_v(inst, "noise1spot", c.spot1);
        self.set_f(inst, "noise1loudness", c.loud1);
        self.set_f(inst, "noise2time", c.time2);
        self.set_v(inst, "noise2spot", c.spot2);
        self.set_f(inst, "noise2loudness", c.loud2);
        if !chosen_slot {
            return;
        }
        let tag = self.propr(a, "Tag").as_name();
        diffuse_noise(&mut NoiseVm { vm: self, a, inst, tag, loc }, loudness);
    }

    /// AActor::execPlayerCanSeeMe. Single-player/client: last render;
    /// server: first PlayerController for which TestCanSeeMe succeeds.
    pub(crate) fn player_can_see_me(&mut self, a: ObjId) -> bool {
        let level = self.native_object(a, crate::NativeField::Level).or(self.level);
        let mode = level.map(|l| self.propr(l, "NetMode").as_int()).unwrap_or(0);
        if mode == 0 || mode == 3 {
            return ((self.level_time - self.native_prop(a, crate::NativeField::LastRenderTime).as_float() as f64) as f32) < 1.0;
        }
        let mut controller = level.and_then(|l| self.po(l, "ControllerList"));
        while let Some(c) = controller {
            let player = self.is_a(c, "PlayerController").then_some(c);
            if self.test_can_see_me(a, player) { return true; }
            controller = self.native_object(c, crate::NativeField::NextController);
        }
        false
    }

    /// AActor::TestCanSeeMe, direct prerequisite of PlayerCanSeeMe.
    /// The square of the dot product accepts both directions, as.
    pub(crate) fn test_can_see_me(&mut self, a: ObjId, player: Option<ObjId>) -> bool {
        let Some(player) = player else { return false };
        let Some(view) = self.po(player, "ViewTarget") else { return false };
        if view == a { return true; }
        let here = self.native_prop(a, crate::NativeField::Location).as_vector();
        let there = self.native_prop(view, crate::NativeField::Location).as_vector();
        let delta = [here[0] - there[0], here[1] - there[1], here[2] - there[2]];
        let distance = (delta[0] * delta[0] + delta[1] * delta[1]) + delta[2] * delta[2];
        let radius = (self.native_prop(a, crate::NativeField::CollisionRadius).as_float() as f64 + 3.6) as f32;
        let limit = fmul_d(radius, 100_000.0);
        if !(limit > distance) { return false; }
        if !self.pb(player, "bBehindView") {
            let direction = rotator_vector(self.propr(player, "Rotation").as_rotator());
            let dot = (delta[1] * direction[1] + delta[2] * direction[2]) + delta[0] * direction[0];
            if !(dot * dot >= distance * 0.25) { return false; }
        }
        self.line_of_sight_to(player, Some(a), false)
    }
}

/// Noise slots (noise1 / noise2) of the Instigator: time, place, volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Slots {
    pub time1: f32,
    pub spot1: V,
    pub loud1: f32,
    pub time2: f32,
    pub spot2: V,
    pub loud2: f32,
}

/// Start of AActor::CheckNoiseHearing: the noise (time `now` = the level's TimeSeconds,
/// double; place `loc`, volume `loudness`) takes a slot of the Instigator; false if a recent,
/// nearby and loud enough slot already covers it (nothing is written, nobody hears).
pub(crate) fn slots_noise(now: f64, loc: V, loudness: f32, c: &mut Slots) -> bool {
    // TimeSeconds (double) − 0.2 and − 0.18: FSUBR rounded to single
    // by the 24-bit FPU before the comparison with the slots' times.
    let old = (now - 0.2f32 as f64) as f32;
    let busy = |t: f32, spot: V, l: f32| {
        let dsq = dist_sq_xyz(sub(spot, loc));
        // (X² + Y²) + Z².
        !(t <= old || 2500.0 <= dsq || loudness * 0.9 > l)
    };
    if busy(c.time1, c.spot1, c.loud1) || busy(c.time2, c.spot2, c.loud2) {
        return false;
    }
    let recent = (now - 0.18f32 as f64) as f32;
    if recent > c.time1 {
        c.time1 = now as f32;
        c.spot1 = loc;
        c.loud1 = loudness;
    } else if recent > c.time2 {
        c.time2 = now as f32;
        c.spot2 = loc;
        c.loud2 = loudness;
    } else {
        let dsq = dist_sq_xyz(sub(c.spot1, loc));
        if (2500.0 <= dsq || c.loud1 > loudness) && c.loud2 > loudness {
            // No weaker slot: nothing is kept.
        } else {
            // The machine code writes noise1 in both cases.
            c.time1 = now as f32;
            c.spot1 = loc;
            c.loud1 = loudness;
        }
    }
    true
}

/// What broadcasting a noise asks of the world (AActor::CheckNoiseHearing, controllers of
/// Level.ControllerList).
pub(crate) trait WorldNoise {
    /// APawn::IsPlayer of the Instigator.
    fn instigator_player(&self) -> bool;
    /// The Instigator's Controller has an Enemy and it is a player.
    fn enemy_player(&self) -> bool;
    fn first_controller(&self) -> Option<ObjId>;
    fn next_one(&self, c: ObjId) -> Option<ObjId>;
    /// The controller's pawn is the Instigator.
    fn pawn_instigator(&self, c: ObjId) -> bool;
    /// IsProbing(HearNoise).
    fn listen(&mut self, c: ObjId) -> bool;
    /// Tag of the controller = Tag of the source.
    fn same_tag(&mut self, c: ObjId) -> bool;
    /// The controller has a pawn and it is a player.
    fn pawn_player(&self, c: ObjId) -> bool;
    /// AController::CanHear.
    fn can_hear_noise(&mut self, c: ObjId, loudness: f32) -> bool;
    /// AController::eventHearNoise.
    fn event_value(&mut self, c: ObjId, loudness: f32);
    fn trace(&mut self, _c: ObjId, _loudness: f32, _heard_by: bool, _heard: bool) {}
}

/// End of AActor::CheckNoiseHearing: HearNoise to the controllers, in list order.
pub(crate) fn diffuse_noise(m: &mut impl WorldNoise, loudness: f32) {
    let restricted = !m.instigator_player() && !m.enemy_player();
    let mut c = m.first_controller();
    let mut kept = 0;
    while let Some(ctrl) = c {
        kept += 1;
        if kept > 10_000 {
            break;
        }
        let next = m.next_one(ctrl);
        if !m.pawn_instigator(ctrl) && m.listen(ctrl) {
            let heard_by = !restricted || m.same_tag(ctrl) || m.pawn_player(ctrl);
            let heard = heard_by && m.can_hear_noise(ctrl, loudness);
            m.trace(ctrl, loudness, heard_by, heard);
            if heard {
                m.event_value(ctrl, loudness);
            }
        }
        c = next;
    }
}

/// Broadcasting a noise in the VM.
struct NoiseVm<'a> {
    vm: &'a mut Vm,
    a: ObjId,
    inst: ObjId,
    tag: String,
    loc: V,
}

impl WorldNoise for NoiseVm<'_> {
    fn instigator_player(&self) -> bool {
        self.vm.pawn_is_player(self.inst)
    }
    fn enemy_player(&self) -> bool {
        self.vm.native_object(self.inst, crate::NativeField::Controller).and_then(|c| self.vm.po(c, "Enemy")).is_some_and(|e| self.vm.pawn_is_player(e))
    }
    fn first_controller(&self) -> Option<ObjId> {
        self.vm.level.and_then(|l| self.vm.po(l, "ControllerList"))
    }
    fn next_one(&self, c: ObjId) -> Option<ObjId> {
        self.vm.native_object(c, crate::NativeField::NextController)
    }
    fn pawn_instigator(&self, c: ObjId) -> bool {
        self.vm.native_object(c, crate::NativeField::Pawn) == Some(self.inst)
    }
    fn listen(&mut self, c: ObjId) -> bool {
        self.vm.is_probing(c, "HearNoise")
    }
    fn same_tag(&mut self, c: ObjId) -> bool {
        self.vm.propr(c, "Tag").as_name().eq_ignore_ascii_case(&self.tag)
    }
    fn pawn_player(&self, c: ObjId) -> bool {
        self.vm.native_object(c, crate::NativeField::Pawn).is_some_and(|p| self.vm.pawn_is_player(p))
    }
    fn can_hear_noise(&mut self, c: ObjId, loudness: f32) -> bool {
        self.vm.can_hear(c, self.loc, loudness, self.a)
    }
    fn event_value(&mut self, c: ObjId, loudness: f32) {
        // AController::eventHearNoise.
        self.vm.event_pub(c, "HearNoise", vec![Value::Float(loudness), Value::Object(Some(self.a))]);
    }
}

