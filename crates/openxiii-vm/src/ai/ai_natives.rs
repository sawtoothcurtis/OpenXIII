//! XIII AI natives recovered from the machine code of XIDPawn.dll.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::{ObjId, Value};
use crate::Vm;

type V = [f32; 3];

/// FVector + FVector of XIDPawn.dll.
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
/// FVector × float of XIDPawn.dll.
fn scale(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn size(a: V) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
/// V × appInvSqrt((X² + Y²) + Z²), without a zero-length test.
fn unsafe_normal(a: V) -> V {
    let r = 1.0 / ((a[0] * a[0] + a[1] * a[1]) + a[2] * a[2]).sqrt();
    scale(a, r)
}

/// Axes (forward, right, up) of an Unreal rotation, like GetAxes: GMath.UnitCoords / R
/// (sines from GMath's table). XIDPawn.dll has its own copy, with the same products:
/// (frame of the rotator) then (product of the frames, for
/// the origin, for each axis).
pub fn axes(r: [i32; 3]) -> [V; 3] {
    let c = openxiii_assets::coords::div_rotator(&openxiii_assets::coords::UNIT, r);
    [c.x, c.y, c.z]
}

impl Vm {
    /// VRand of XIDPawn.dll: (appFrand + appFrand) − 1 per component, drawn
    /// again while (Z² + Y²) + X² > 1, then (without a zero-length test).
    fn vrand(&mut self) -> V {
        loop {
            let p = [self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0];
            if (p[2] * p[2] + p[1] * p[1]) + p[0] * p[0] <= 1.0 {
                return unsafe_normal(p);
            }
        }
    }

    /// AIAController::execDirectionDuTir (XIDPawn.dll): point the soldier aims at, kept in
    /// DirectionTir; also computes WeaponStartTrace (eyes + the weapon's FireOffset, +16 in front for a hand-held
    /// bullet weapon). Level 5: one time in two, perfect shot (centre of the enemy, or lead
    /// for a projectile). Otherwise: the enemy's last known position (EnemyTargetPos) led by its velocity
    /// (and that of its support), plus an error perpendicular to the aim of dist × tan(angle), the angle
    /// being Angle_Visee (bTirSurConeMax) or a random fraction; height corrected by level
    /// (1: -35, 2-3: -25, others: +23.62); lower if the enemy is crouched.
    pub fn direction_shot(&mut self, c: ObjId) -> V {
        let Some(soldier) = self.propr(c, "BaseS").as_object() else {
            return [0.0; 3];
        };
        let target = self.propr(c, "EnemyTargetPos").as_vector();
        let Some(pawn) = self.propr(c, "Pawn").as_object() else {
            return [0.0; 3];
        };
        if target == [0.0; 3] {
            let name = self.objects[pawn].as_ref().map(|o| o.name.clone()).unwrap_or_default();
            self.log.push(format!("{name}    WUUUUUUUUUU WUUUUUUUUU WUUUUUUUU  EnemyTarget POS = 0 0 0 PAS BON BUG BUGGGGG"));
            return [0.0; 3];
        }
        let Some(enemy) = self.propr(c, "Enemy").as_object() else {
            return [0.0; 3];
        };
        let weapon = self.propr(pawn, "Weapon").as_object();
        let ammo = weapon.and_then(|w| self.propr(w, "AmmoType").as_object());
        let instant = ammo.is_some_and(|a| self.propr(a, "bInstantHit").as_bool());
        let group = weapon.map(|w| self.propr(w, "InventoryGroup").as_byte()).unwrap_or(0);
        let [x, y, z] = axes(self.propr(c, "Rotation").as_rotator());
        let here = self.propr(pawn, "Location").as_vector();
        let off = weapon.map(|w| self.propr(w, "FireOffset").as_vector()).unwrap_or([0.0; 3]);
        let mut start = add(here, [0.0, 0.0, self.propr(pawn, "BaseEyeHeight").as_float()]);
        start = add(add(add(start, scale(x, off[0])), scale(y, off[1])), scale(z, off[2]));
        let hand = weapon.map(|w| self.propr(w, "WHand").as_byte()).unwrap_or(0);
        if instant && hand != 0 && hand != 4 {
            start = add(start, scale(x, 16.0));
        }
        self.set_prop(c, "WeaponStartTrace", Value::vector(start[0], start[1], start[2]));
        let enemy_at = self.propr(enemy, "Location").as_vector();
        let dist = size([enemy_at[0] - here[0], enemy_at[1] - here[1], enemy_at[2] - here[2]]);
        let skill = self.propr(soldier, "Skill").as_int();
        // Default projectile speed of AmmoType.ProjectileClass (GetDefaultActor).
        let projectile_speed = || -> f32 {
            let class = ammo.and_then(|a| self.propr(a, "ProjectileClass").as_class());
            // Zero speed: division by zero as in the original.
            class.map(|k| self.propr(self.classes[k].cdo, "Speed").as_float()).unwrap_or(1.0)
        };
        let speed = projectile_speed();
        // dist × 0.2778 − 133.33: double constants, 24-bit FPU (each result rounded).
        // A single 24-bit rounding per x87 operation (fmul / fsub qword): no double rounding
        // by going through f64.
        let lob = || x87_d24(x87_d24(dist as f64, 0.2778, false) as f64, -133.33, true);
        let result = if skill == 5 && self.rand() < 0.5 {
            if instant {
                enemy_at
            } else if group == 4 {
                [enemy_at[0], enemy_at[1], lob() + enemy_at[2]]
            } else {
                add(enemy_at, scale(self.propr(enemy, "Velocity").as_vector(), dist / speed))
            }
        } else {
            let cone = self.propr(c, "Angle_Visee").as_float();
            let a = unsafe_normal(x);
            let r = self.vrand();
            // R × A, normalized.
            let v = unsafe_normal([r[1] * a[2] - r[2] * a[1], r[2] * a[0] - a[2] * r[0], a[1] * r[0] - r[1] * a[0]]);
            let angle = if self.propr(c, "bTirSurConeMax").as_bool() { cone } else { self.rand() * cone };
            // appTan(angle × 0.01745) in double (the code's constant, not π / 180).
            let rad = (angle as f64 * 0.01745) as f32;
            // fptan is not rounded by the precision control (24 bits only applies to
            // fadd/fmul/fdiv/fsqrt): the tangent is kept in double, only the
            // product by the distance is rounded.
            let k = (dist as f64 * (rad as f64).dtan()) as f32;
            let mut e = scale(v, k);
            e[2] = match skill {
                1 => e[2] - 35.0,
                2 | 3 => e[2] - 25.0,
                _ => e[2] + 23.62,
            };
            let refresh = self.propr(c, "Temps_RefreshEnemyPos").as_float();
            let velocity = self.propr(c, "EnemyTargetVelocity").as_vector();
            let base_velocity = self.propr(enemy, "Base").as_object().map(|b| self.propr(b, "Velocity").as_vector());
            // Grenades (group 4, and 20 on a support): target + lob + half speed, without the aiming
            // error.
            let mut point = match (base_velocity, instant) {
                (Some(bv), true) => add(add(e, target), scale(add(bv, velocity), refresh)),
                (Some(bv), false) if group != 4 && group != 20 => add(add(e, target), scale(add(bv, velocity), dist / speed + refresh)),
                (Some(bv), false) => {
                    let h = add(scale(velocity, 0.5), bv);
                    let t = dist / speed + refresh;
                    add([target[0], target[1], lob() + target[2]], scale(h, t))
                }
                (None, true) => add(add(e, target), scale(velocity, refresh)),
                (None, false) if group == 4 => {
                    let t = dist / speed + refresh;
                    add([target[0], target[1], lob() + target[2]], scale(scale(velocity, 0.5), t))
                }
                (None, false) => add(add(e, target), scale(velocity, dist / speed + refresh)),
            };
            if self.propr(enemy, "bIsCrouched").as_bool() {
                point[2] -= self.propr(pawn, "CollisionHeight").as_float() - self.propr(pawn, "CrouchHeight").as_float();
            }
            point
        };
        self.set_prop(c, "DirectionTir", Value::vector(result[0], result[1], result[2]));
        result
    }

    /// AIAController::execFindNewStakeOutDir (XIDPawn.dll): among the navigation
    /// points more than 100 and less than 800 units from the pawn, the one most in the direction of
    /// the enemy (dot product > the best, −1 at the start) and in sight (LineOfSightTo) gives
    /// LastSeenPos (X, Y of the point, Z + CollisionHeight × 0.5).
    pub fn find_new_stakeout_dir(&mut self, c: ObjId) {
        let (Some(enemy), Some(pawn)) = (self.po(c, "Enemy"), self.native_object(c, crate::NativeField::Pawn)) else {
            return;
        };
        let here = self.propr(pawn, "Location").as_vector();
        let e = self.propr(enemy, "Location").as_vector();
        // (UnsafeNormal): appInvSqrt((X² + Y²) + Z²).
        let d = [e[0] - here[0], e[1] - here[1], e[2] - here[2]];
        let r = 1.0 / ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]).sqrt();
        let to_enemy = [r * d[0], r * d[1], r * d[2]];
        let mut best: Option<ObjId> = None;
        let mut best_dot = -1.0f32;
        let mut nav = self.level.and_then(|l| self.propr(l, "NavigationPointList").as_object());
        let mut guard = 0;
        while let Some(n) = nav {
            guard += 1;
            if guard > 100_000 {
                break;
            }
            let here = self.propr(pawn, "Location").as_vector();
            let p = self.propr(n, "Location").as_vector();
            let d = [p[0] - here[0], p[1] - here[1], p[2] - here[2]];
            let dist = (((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]) as f64).sqrt() as f32;
            if dist < 800.0 && dist > 100.0 {
                let inv = 1.0 / dist;
                let u = [d[0] * inv, d[1] * inv, d[2] * inv];
                let dot = (to_enemy[2] * u[2] + to_enemy[1] * u[1]) + to_enemy[0] * u[0];
                let mut seen_flag = None;
                if dot > best_dot {
                    seen_flag = Some(self.line_of_sight_to(c, Some(n), false));
                }
                if seen_flag == Some(true) {
                    best_dot = dot;
                    best = Some(n);
                }
            }
            nav = self.propr(n, "nextNavigationPoint").as_object();
        }
        if let Some(b) = best {
            let p = self.propr(b, "Location").as_vector();
            let half = (self.propr(pawn, "CollisionHeight").as_float() as f64 * 0.5) as f32;
            self.set_prop(c, "LastSeenPos", Value::vector(p[0], p[1], half + p[2]));
        }
    }

    /// AIAController::execPickStartPoint (XIDPawn.dll): in GameInfo.PatrolPointList,
    /// the patrol point of the soldier's network (NumReseauPropre; otherwise those of network 0) at most
    /// 780 units away (608 400 squared) and reachable (APawn::actorReachable), the closest one;
    /// the first candidate is always taken, the best distance starts at 1000 and is not
    /// reset between the two passes.
    pub fn pick_start_point(&mut self, c: ObjId) -> Option<ObjId> {
        self.pick_start_point_with(c, |vm, pawn, p| vm.actor_reachable(pawn, Some(p), false, false))
    }

    /// `pick_start_point` with the given reachability test (tests can supply their own).
    pub(crate) fn pick_start_point_with(&mut self, c: ObjId, mut reachable: impl FnMut(&mut Vm, ObjId, ObjId) -> bool) -> Option<ObjId> {
        let soldier = self.po(c, "BaseS")?;
        let pawn = self.native_object(c, crate::NativeField::Pawn)?;
        let game = self.level.and_then(|l| self.po(l, "Game"))?;
        let Value::Array(points) = self.prop(game, "PatrolPointList") else {
            return None;
        };
        let network = self.propr(soldier, "NumReseauPropre").as_int();
        let mut best: Option<ObjId> = None;
        let mut best_d2 = 1000.0f32;
        let passes: &[i32] = if network != 0 { &[network, 0] } else { &[0] };
        for &wanted in passes {
            for p in points.iter().filter_map(|p| p.as_object()) {
                if self.objects[p].is_none() || self.propr(p, "NumReseau").as_int() != wanted {
                    continue;
                }
                let (l, h) = (self.propr(p, "Location").as_vector(), self.propr(pawn, "Location").as_vector());
                let d = [l[0] - h[0], l[1] - h[1], l[2] - h[2]];
                if (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2] > 608_400.0 || !reachable(self, pawn, p) {
                    continue;
                }
                let h = self.propr(pawn, "Location").as_vector();
                let e = [h[0] - l[0], h[1] - l[1], h[2] - l[2]];
                // Distance kept: Z² + Y² then + X² (x87 stack loaded in the order X, Y, Z).
                let d2 = (e[2] * e[2] + e[1] * e[1]) + e[0] * e[0];
                if best.is_none() || best_d2 > d2 {
                    best_d2 = d2;
                    best = Some(p);
                }
            }
            if best.is_some() {
                return best;
            }
        }
        best
    }
}

/// 24-bit x87 operation with a double operand (fmul / fadd qword): `a × b`, or `a + b`
/// if `sum_value`, rounded only once to the nearest float (ties to even), like the FPU that computes the
/// exact result. The exact error term (FMA or TwoSum) breaks the f64 ties.
pub(crate) fn x87_d24(a: f64, b: f64, sum_value: bool) -> f32 {
    let (p, err) = if sum_value {
        let p = a + b;
        let bb = p - a;
        (p, (a - (p - bb)) + (b - bb))
    } else {
        let p = a * b;
        (p, a.mul_add(b, -p))
    };
    let r = p as f32;
    if err == 0.0 || !p.is_finite() {
        return r;
    }
    let back = r as f64;
    if back == p {
        return r;
    }
    let (lo, hi) = if back < p { (r, r.next_up()) } else { (r.next_down(), r) };
    let mid = (lo as f64 + hi as f64) / 2.0;
    if p == mid {
        return if err > 0.0 { hi } else { lo };
    }
    r
}

impl Vm {
    /// APawn::execEyePosition: (0, 0, EyeHeight); ABaseSoldier::execEyePosition
    /// (0, 0, BaseEyeHeight).
    pub(crate) fn eye_position(&self, a: ObjId) -> [f32; 3] {
        [0.0, 0.0, self.propr(a, if self.is_a(a, "BaseSoldier") { "BaseEyeHeight" } else { "EyeHeight" }).as_float()]
    }
}

/// a − b, component by component (24-bit FSUB = f32 subtraction).
pub(crate) fn vect_minus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// a × s (tests only: our code does not use it yet).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn vect_times(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// SizeSquared, (X² + Y²) + Z², left on the x87 stack (24-bit mantissa,
/// extended exponent: beyond f32::MAX, the original keeps a finite value that we return as
/// infinite; its callers store it as f32 or compare it with another, with no effect in game).
pub(crate) fn size2(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1]) + a[2] * a[2]
}

