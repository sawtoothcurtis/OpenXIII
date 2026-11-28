//! BotController natives (XIIIMP.dll): bots' order list, search for pickups,
//! hiding places and watch points, paths with tolerance. XIIIMP.dll contains nothing other
//! than these 19 natives and TheDuck's PreNetReceive / PostNetReceive (network); the rest of the AI of the
//! bots and of the DM, CTF and Sabotage modes is in UnrealScript. Addresses at the top of each function.
//!
//! Distances are computed in f32 (the game's FPU at 24-bit precision) and weights go through
//! _ftol (truncation).

use crate::physics::CheckResult;
use crate::value::{ObjId, Value};
use crate::Vm;

type V = [f32; 3];

/// TRACE_World | TRACE_StopAtFirstHit.
const TRACE_WORLD_STOP: u32 = 0x286;

/// Order (struct BotController.Order, 0x18 bytes): Priority, TypeId, TypeName, ExcludeIds.
#[derive(Debug, Clone, Default)]
struct Order {
    priority: i32,
    type_id: i32,
    type_name: String,
    exclude_ids: i32,
}

fn dist(a: V, b: V) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Distance whose sum of squares follows another order of the machine code (`order`: components
/// in addition order, e.g. [2, 1, 0] for (dZ² + dY²) + dX²).
fn dist_in(a: V, b: V, order: [usize; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let [i, j, k] = order;
    (d[i] * d[i] + d[j] * d[j] + d[k] * d[k]).sqrt()
}

/// XIIIMP's _ftol (a copy of msvcr70's): fistp qword, the caller keeps EAX, the
/// low 32 bits of the 64-bit truncation.
fn ftol(f: f32) -> i32 {
    crate::value::ftol(f) as i32
}

impl Vm {
    fn order_from(v: &Value) -> Order {
        match v {
            Value::Struct(_, f) if f.len() >= 4 => Order { priority: f[0].as_int(), type_id: f[1].as_int(), type_name: f[2].as_name(), exclude_ids: f[3].as_int() },
            _ => Order::default(),
        }
    }

    fn order_value(&self, c: ObjId, o: &Order) -> Value {
        let name = match self.prop(c, "CurrentOrder") {
            Value::Struct(n, _) => n,
            _ => "botcontroller.order".into(),
        };
        Value::Struct(name, vec![Value::Int(o.priority), Value::Int(o.type_id), Value::Str(o.type_name.clone()), Value::Int(o.exclude_ids)].into())
    }

    fn order_list(&self, c: ObjId) -> Vec<Order> {
        match self.prop(c, "OrderList") {
            Value::Array(a) => a.iter().map(Self::order_from).collect(),
            _ => Vec::new(),
        }
    }

    fn set_order_list(&mut self, c: ObjId, list: &[Order]) {
        let values = list.iter().map(|o| self.order_value(c, o)).collect();
        self.set_prop(c, "OrderList", Value::Array(values));
    }

    fn current_order(&self, c: ObjId) -> Order {
        Self::order_from(&self.prop(c, "CurrentOrder"))
    }

    fn set_current_order(&mut self, c: ObjId, o: &Order) {
        let v = self.order_value(c, o);
        self.set_prop(c, "CurrentOrder", v);
    }

    fn order_number(&self, c: ObjId) -> i32 {
        self.propr(c, "OrderNumber").as_int()
    }

    fn actor_list(&self, c: ObjId, name: &str) -> Vec<Option<ObjId>> {
        match self.prop(c, name) {
            Value::Array(a) => a.iter().map(|v| v.as_object()).collect(),
            _ => Vec::new(),
        }
    }

    /// ABotController::eventChangeState and the like: ProcessEvent of the
    /// script function (in the current state).
    fn bot_event(&mut self, c: ObjId, name: &str) {
        self.event_pub(c, name, Vec::new());
    }

    /// ABotController::FindMostImportantOrder: CurrentOrder becomes the first order
    /// with the highest priority (strictly positive).
    fn bot_find_most_important_order(&mut self, c: ObjId) {
        let mut best_priority = 0;
        let mut best = Order::default();
        let list = self.order_list(c);
        for o in list.iter().take(self.order_number(c).max(0) as usize) {
            if best_priority < o.priority {
                best_priority = o.priority;
                best = o.clone();
            }
        }
        self.set_current_order(c, &best);
    }

    /// ABotController::RemoveOrder: removes the first order of this type; with no order
    /// left, AddDefaultOrders; if it was the current order, the most important one replaces it and
    /// ChangeState.
    fn bot_remove_order(&mut self, c: ObjId, id: i32) {
        let current = self.current_order(c).type_id;
        let mut list = self.order_list(c);
        let n = self.order_number(c);
        if let Some(i) = (0..n.max(0) as usize).find(|&i| list.get(i).is_some_and(|o| o.type_id == id)) {
            list.remove(i);
            self.set_order_list(c, &list);
            self.set_prop(c, "OrderNumber", Value::Int(n - 1));
        }
        if self.pb(c, "DBugOrder") {
            self.bot_event(c, "LogOrderList");
        }
        if self.order_number(c) == 0 {
            self.bot_event(c, "AddDefaultOrders");
        } else if id == current {
            self.bot_find_most_important_order(c);
            self.bot_event(c, "ChangeState");
        }
    }

    /// ABotController::ExcludeOrders: RemoveOrder of each bit of ExcludeIds
    /// (64 iterations, the mask drops back to zero after the 32nd bit).
    fn bot_exclude_orders(&mut self, c: ObjId, exclude: i32) {
        if exclude == 0 {
            return;
        }
        let mut bit: u32 = 1;
        for _ in 0..0x40 {
            if exclude as u32 & bit != 0 {
                self.bot_remove_order(c, bit as i32);
            }
            bit = bit.wrapping_mul(2);
        }
    }

    /// ABotController::execAddOrder.
    fn bot_add_order(&mut self, c: ObjId, priority: i32, type_id: i32, type_name: String, exclude: i32) {
        let id = self.propr(c, "ID").as_int();
        if priority == 0 {
            if self.pb(c, "DBugBot") {
                self.log.push(format!("Log: [ WARNING ][ AddOrder ][ BOT {id} ] : Try To Add New Order With Priority 0 !..."));
            }
            return;
        }
        let mut list = self.order_list(c);
        let n = self.order_number(c);
        if list.iter().take(n.max(0) as usize).any(|o| o.type_id == type_id) {
            if self.pb(c, "DBugBot") {
                // TypeName of the requested order, passed by value to Logf ("%s"): empty, its Data is null and
                // vsnprintf writes "(null)".
                let name_value = if type_name.is_empty() { "(null)" } else { type_name.as_str() };
                self.log.push(format!("Log: [ WARNING ][ AddOrder ][ BOT {id} ] : Try To Add Order Than Already Exist ({name_value}) !..."));
            }
            return;
        }
        let n = n + 1;
        self.set_prop(c, "OrderNumber", Value::Int(n));
        let new = Order { priority, type_id, type_name, exclude_ids: exclude };
        // FArray::AddZeroed then write at index OrderNumber - 1.
        list.push(Order::default());
        if let Some(slot) = list.get_mut((n - 1).max(0) as usize) {
            *slot = new.clone();
        }
        self.set_order_list(c, &list);
        if self.pb(c, "DBugOrder") {
            self.bot_event(c, "LogOrderList");
        }
        if n == 1 || self.current_order(c).priority < priority {
            self.set_current_order(c, &new);
            self.bot_exclude_orders(c, exclude);
            self.bot_event(c, "ChangeState");
        }
    }

    /// ABotController::execModifyOrder.
    fn bot_modify_order(&mut self, c: ObjId, id: i32, priority: i32) {
        if priority == 0 {
            if self.pb(c, "DBugBot") {
                let bot = self.propr(c, "ID").as_int();
                self.log.push(format!("Log: [ WARNING ][ ModifyOrder ][ BOT {bot} ] : Try To Modify Order With Priority 0 !..."));
            }
            return;
        }
        let was_current = id == self.current_order(c).type_id;
        let mut list = self.order_list(c);
        let n = self.order_number(c).max(0) as usize;
        if let Some(o) = list.iter_mut().take(n).find(|o| o.type_id == id) {
            o.priority = priority;
            self.set_order_list(c, &list);
        }
        if self.pb(c, "DBugOrder") {
            self.bot_event(c, "LogOrderList");
        }
        if was_current || self.current_order(c).priority < priority {
            self.bot_find_most_important_order(c);
            self.bot_event(c, "ChangeState");
        }
    }

    /// ABotController::execFindOrder.
    fn bot_find_order(&self, c: ObjId, id: i32) -> bool {
        let n = self.order_number(c).max(0) as usize;
        self.order_list(c).iter().take(n).any(|o| o.type_id == id)
    }

    /// ABotController::execClearAllOrders.
    fn bot_clear_all_orders(&mut self, c: ObjId) {
        self.set_prop(c, "OrderList", Value::Array(Vec::new()));
        self.set_prop(c, "OrderNumber", Value::Int(0));
        self.bot_event(c, "AddDefaultOrders");
    }

    /// ABotController::CalcPathWeight: length of the RouteCache from the pawn
    /// (sum in f32, then _ftol).
    fn bot_calc_path_weight(&self, c: ObjId) -> i32 {
        let Some(p) = self.native_object(c, crate::NativeField::Pawn) else { return 0 };
        let cache = match self.prop(c, "RouteCache") {
            Value::Array(a) => a,
            _ => Vec::new(),
        };
        let mut at = self.native_prop(p, crate::NativeField::Location).as_vector();
        let mut weight = 0.0f32;
        for n in cache.iter().take(16) {
            let Some(n) = n.as_object() else { break };
            let l = self.native_prop(n, crate::NativeField::Location).as_vector();
            // |L − At| inline: X, Y then Z loaded, (Z² + Y²) + X², appSqrt in double.
            let d = [l[0] - at[0], l[1] - at[1], l[2] - at[2]];
            weight += (((d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]) as f64).sqrt() as f32;
            at = l;
        }
        ftol(weight)
    }

    /// ABotController::execExtendFindPathToward: refuses a goal with no ground within
    /// HeightMargin + 78 below it (downward trace), then FindPath towards its position with
    /// GReachedDestinationXYMargin / HeightMargin and MoveTarget reset to None.
    fn bot_extend_find_path_toward(&mut self, c: ObjId, desired: Option<ObjId>, xy: f32, height: f32) -> Option<ObjId> {
        let (Some(d), Some(_)) = (desired, self.native_object(c, crate::NativeField::Pawn)) else {
            self.log.push("Log: GROSSSSSSSSSSSSSSSSSS BUGGGGGGGGGGGGGGGGGG ACTOR==NONE !!!!!!!!!!!!!!!!!!!!!!".into());
            return None;
        };
        let at = self.native_prop(d, crate::NativeField::Location).as_vector();
        let below = [at[0], at[1], at[2] - (height + 78.0)];
        let mut hit = CheckResult::new(1.0);
        self.single_line_check(&mut hit, Some(c), below, at, TRACE_WORLD_STOP, [0.0; 3]);
        if hit.actor.is_none() {
            if self.pb(c, "DBugBot") {
                let name = self.objects[d].as_ref().map_or(String::new(), |o| o.name.clone());
                self.log.push(format!("Log: Pas bon Trop Haut ({name})"));
            }
            return None;
        }
        self.reach_xy_margin = xy;
        self.reach_height_margin = height;
        self.set_prop(c, "MoveTarget", Value::Object(None));
        let found = self.find_path(c, at, None, true);
        self.reach_xy_margin = 0.0;
        self.reach_height_margin = 0.0;
        found
    }

    /// ABotController::execSearchHidePoint: from a randomly drawn index in
    /// FullPickUpList, the first object more than 1200 units away or hidden from FearActor (trace) and
    /// reachable becomes HidePoint (MoveTarget, the first point of the path); otherwise the first
    /// navigation point that fits.
    fn bot_search_hide_point(&mut self, c: ObjId) {
        self.set_prop(c, "HidePoint", Value::Object(None));
        let Some(fear) = self.po(c, "FearActor") else { return };
        let pickups = self.actor_list(c, "FullPickUpList");
        let r = self.rand();
        let mut i = ftol(pickups.len() as f32 * r);
        while (i as usize) < pickups.len() && i >= 0 {
            if self.native_object(c, crate::NativeField::Pawn).is_none() {
                break;
            }
            if let Some(item) = pickups[i as usize] {
                if self.bot_hide_candidate(c, fear, item, true) {
                    return;
                }
            }
            i += 1;
        }
        let mut n = self.level.and_then(|l| self.po(l, "NavigationPointList"));
        while let Some(nav) = n {
            if self.native_object(c, crate::NativeField::Pawn).is_none() {
                return;
            }
            if self.bot_hide_candidate(c, fear, nav, false) {
                return;
            }
            n = self.po(nav, "nextNavigationPoint");
        }
    }

    /// Distance to the pawn: (dX² + dZ²) + dY² for an object, (dX² + dY²) + dZ² for
    /// a navigation point.
    fn bot_hide_candidate(&mut self, c: ObjId, fear: ObjId, spot: ObjId, pickup: bool) -> bool {
        let Some(p) = self.native_object(c, crate::NativeField::Pawn) else { return false };
        let at = self.native_prop(spot, crate::NativeField::Location).as_vector();
        let d = if pickup { dist_in(at, self.native_prop(p, crate::NativeField::Location).as_vector(), [0, 2, 1]) } else { dist(at, self.native_prop(p, crate::NativeField::Location).as_vector()) };
        if ftol(d) < 0x4b1 {
            let mut hit = CheckResult::new(1.0);
            let from = self.native_prop(fear, crate::NativeField::Location).as_vector();
            self.single_line_check(&mut hit, Some(c), from, at, TRACE_WORLD_STOP, [0.0; 3]);
            if hit.actor.is_none() {
                return false;
            }
        }
        if let Some(first) = self.find_path(c, [0.0; 3], Some(spot), true) {
            self.set_prop(c, "HidePoint", Value::Object(Some(spot)));
            self.set_prop(c, "MoveTarget", Value::Object(Some(first)));
            return true;
        }
        false
    }

    /// ABotController::FindMedKit, FindArmor and FindWeapon
    /// visible object of the list, within MaxSeekDist (if `limited`), reachable
    /// and with the shortest path (RouteDist); "best" (armour, weapon): the best ratio
    /// (11 - MaxDesireability) / (3 × RouteDist). FindWeapon for the nearest only keeps the weapons that
    /// respawn in less than 9 s. Item and MoveTarget take the object and the first point.
    fn bot_find_item(&mut self, c: ObjId, kind: Seek, limited: bool, best: bool) {
        let (list, max_name, error_bit, what) = match kind {
            Seek::Life => ("MedKitList", "MaxSeekDist_Life", "PathErrorToAllLife", "LIFE"),
            Seek::Armor => ("ArmorList", "MaxSeekDist_Armor", "PathErrorToAllArmor", "ARMOR"),
            Seek::Weapon => ("WeaponList", "MaxSeekDist_Weapon", "PathErrorToAllWeapon", "WEAPON"),
        };
        self.set_prop(c, "Item", Value::Object(None));
        let (mut tried, mut failed, mut too_far) = (0, 0, 0);
        let mut best_int = 0i32;
        let mut best_float = 0.0f32;
        let max = self.propr(c, max_name).as_int();
        let items = self.actor_list(c, list);
        for item in items.into_iter().flatten() {
            if self.native_prop(item, crate::NativeField::Hidden).as_bool() {
                continue;
            }
            let Some(p) = self.native_object(c, crate::NativeField::Pawn) else { break };
            if limited && dist(self.native_prop(item, crate::NativeField::Location).as_vector(), self.native_prop(p, crate::NativeField::Location).as_vector()) >= max as f32 {
                too_far += 1;
                continue;
            }
            tried += 1;
            let Some(first) = self.find_path(c, [0.0; 3], Some(item), true) else {
                failed += 1;
                continue;
            };
            let weight = ftol(self.pf(c, "RouteDist"));
            let none = self.po(c, "Item").is_none();
            // Pickup+0x1F4 (MaxDesireability), read after Cast<APickup>.
            let wanted = || self.pf(item, "MaxDesireability");
            let take = match kind {
                Seek::Life => {
                    if none || weight < best_int {
                        best_int = weight;
                        true
                    } else {
                        false
                    }
                }
                _ if best => {
                    let score = (11.0 - wanted()) / (weight.wrapping_mul(3)) as f32;
                    if none || best_float < score {
                        best_float = score;
                        true
                    } else {
                        false
                    }
                }
                _ => {
                    let w = weight as f32;
                    let ok = (none || w < best_float) && (kind == Seek::Armor || wanted() < 9.0);
                    if ok {
                        best_float = w;
                    }
                    ok
                }
            };
            if take {
                self.set_prop(c, "Item", Value::Object(Some(item)));
                self.set_prop(c, "MoveTarget", Value::Object(Some(first)));
            }
        }
        if self.po(c, "Item").is_some() {
            return;
        }
        let id = self.propr(c, "ID").as_int();
        if tried == failed && tried > 0 {
            if self.pb(c, "DBugBot") || self.pb(c, "DBugWarning") {
                self.log.push(format!("Log: [ ERROR ][ {what} ][ BOT {id} ] : All PathFinding Failed !..."));
            }
            self.set_b(c, error_bit, true);
        } else if too_far > 0 {
            let opt = match kind {
                Seek::Life => "Life",
                Seek::Armor => "Armor",
                Seek::Weapon => "Weapon",
            };
            let tag = if kind == Seek::Life { "Life" } else { what };
            self.log.push(format!("Log: [ WARNING ][ {tag} ][ BOT {id} ] : Seek Dist({max}) Opt For {opt} Too Small !..."));
        } else if self.pb(c, "DBugBot") {
            // FindMedKit's message talks about weapons (string).
            let (tag, things) = match kind {
                Seek::Life => ("WEAPON", "MedKit"),
                Seek::Armor => ("ARMOR", "Armor"),
                Seek::Weapon => ("WEAPON", "Weapon"),
            };
            self.log.push(format!("Log: [ WARNING ][ {tag} ][ BOT {id} ] : All {things} Are Unpickable !..."));
        }
    }

    /// ABotController::execFindAllNearestWatchPoint: UseableWatchPointList receives
    /// the points of WatchPointList within 3000 units of CenterActor and in sight of it.
    fn bot_find_all_nearest_watch_point(&mut self, c: ObjId, center: Option<ObjId>) {
        let Some(center) = center else { return };
        let mut usable = Vec::new();
        for wp in self.actor_list(c, "WatchPointList").into_iter().flatten() {
            let at = self.native_prop(wp, crate::NativeField::Location).as_vector();
            let from = self.native_prop(center, crate::NativeField::Location).as_vector();
            if ftol(dist(at, from)) < 3000 {
                let mut hit = CheckResult::new(1.0);
                self.single_line_check(&mut hit, Some(c), from, at, TRACE_WORLD_STOP, [0.0; 3]);
                if hit.actor.is_none() {
                    usable.push(Value::Object(Some(wp)));
                }
            }
        }
        self.set_prop(c, "UseableWatchPointList", Value::Array(usable));
    }

    pub(crate) fn bot_native(&mut self, name: &str, this: ObjId, args: &mut [Value], supplied: &[bool]) -> Option<Value> {
        if !self.is_a(this, "BotController") {
            return None;
        }
        let int = |i: usize| args.get(i).map_or(0, |v| v.as_int());
        let obj = |i: usize| args.get(i).and_then(|v| v.as_object());
        let given = |i: usize| supplied.get(i).copied().unwrap_or(false);
        Some(match name {
            // ABotController::execStopMvtWhenMayFall: GetStateFrame()->LatentAction = 0.
            "StopMvtWhenMayFall" => {
                self.set_latent_action(this, 0);
                Value::Void
            }
            "ExtendFindPathToward" => {
                let xy = if given(1) { args[1].as_float() } else { 70.0 };
                let h = if given(2) { args[2].as_float() } else { 160.0 };
                Value::Object(self.bot_extend_find_path_toward(this, obj(0), xy, h))
            }
            // ABotController::execCalcPathWeight.
            "CalcPathWeight" => Value::Int(self.bot_calc_path_weight(this)),
            "FindOrder" => Value::Bool(self.bot_find_order(this, int(0))),
            // ABotController::execFindMostImportantOrder.
            "FindMostImportantOrder" => {
                self.bot_find_most_important_order(this);
                Value::Void
            }
            // ABotController::execRemoveOrder.
            "RemoveOrder" => {
                self.bot_remove_order(this, int(0));
                Value::Void
            }
            // ABotController::execExcludeOrders.
            "ExcludeOrders" => {
                self.bot_exclude_orders(this, int(0));
                Value::Void
            }
            "AddOrder" => {
                let type_name = if given(2) { args[2].as_name() } else { String::new() };
                self.bot_add_order(this, int(0), int(1), type_name, int(3));
                Value::Void
            }
            "ModifyOrder" => {
                self.bot_modify_order(this, int(0), int(1));
                Value::Void
            }
            "ClearAllOrders" => {
                self.bot_clear_all_orders(this);
                Value::Void
            }
            "SearchHidePoint" => {
                self.bot_search_hide_point(this);
                Value::Void
            }
            // execFindMedKit: FindMedKit(true).
            "FindMedKit" => {
                self.bot_find_item(this, Seek::Life, true, false);
                Value::Void
            }
            // execFindNearestArmor / execFindBestArmor.
            "FindNearestArmor" | "FindBestArmor" => {
                self.bot_find_item(this, Seek::Armor, true, name == "FindBestArmor");
                Value::Void
            }
            // execFindNearestWeapon / execFindBestWeapon.
            "FindNearestWeapon" | "FindBestWeapon" => {
                self.bot_find_item(this, Seek::Weapon, true, name == "FindBestWeapon");
                Value::Void
            }
            // ABotController::execIsNearest: true if Enemy2 is closer to the pawn
            // than Enemy1.
            "IsNearest" => {
                let (Some(e1), Some(e2), Some(p)) = (obj(0), obj(1), self.native_object(this, crate::NativeField::Pawn)) else {
                    return Some(Value::Bool(false));
                };
                let here = self.native_prop(p, crate::NativeField::Location).as_vector();
                Value::Bool(dist(self.native_prop(e2, crate::NativeField::Location).as_vector(), here) < dist(self.native_prop(e1, crate::NativeField::Location).as_vector(), here))
            }
            // ABotController::execIsInAttackArea: distance to the pawn ≤ AttackArea.
            "IsInAttackArea" => {
                let (Some(e), Some(p)) = (obj(0), self.native_object(this, crate::NativeField::Pawn)) else {
                    return Some(Value::Bool(false));
                };
                Value::Bool(dist(self.native_prop(e, crate::NativeField::Location).as_vector(), self.native_prop(p, crate::NativeField::Location).as_vector()) <= self.propr(this, "AttackArea").as_int() as f32)
            }
            "FindAllNearestWatchPoint" => {
                self.bot_find_all_nearest_watch_point(this, obj(0));
                Value::Void
            }
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seek {
    Life,
    Armor,
    Weapon,
}

