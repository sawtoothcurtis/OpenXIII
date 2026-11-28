//! Player aiming, reimplemented from Engine.dll: APlayerController::execAdjustAimForDisplay
//! and APlayerController::SmoothedAim. The target under the crosshair (or, with
//! iAutoAimMode, the best visible pawn in a cone) gives the firing direction
//! (AdjustedAimForFiring, with projectile lead) and the displayed aim (ViewAdjustAim).

use crate::physics::{vector_rotation, CheckResult, V};
use crate::value::{ObjId, Value};
use crate::Vm;

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// FVector::SafeNormal: zero below 1e-8.
fn safe_normal(v: V) -> V {
    crate::natives::safe_normal(v)
}

impl Vm {
    /// APlayerController::SmoothedAim: ViewAdjustAim slides towards the kept aim
    /// (MemAim) when a target is held (bLocked), and comes back otherwise; the returned rotator keeps the
    /// controller's pitch.
    pub(crate) fn smoothed_aim(&mut self, pc: ObjId, r: [i32; 3]) -> [i32; 3] {
        let step = self.propr(pc, "DeltaT").as_float() * 4.0;
        let old = self.pv3(pc, "OldAdjustAim");
        self.set_v3(pc, "ViewAdjustAim", old);
        let r = [self.propr(pc, "Rotation").as_rotator()[0], r[1], r[2]];
        let old = crate::ai_move::rotator_vector(r);
        self.set_v3(pc, "OldAdjustAim", old);
        let target = self.propr(pc, "BestTarget").as_object();
        let locked = self.propr(pc, "bLocked").as_bool();
        if target.is_some() && !locked {
            let v = self.pv3(pc, "ViewAdjustAim");
            self.set_v3(pc, "MemAim", v);
            self.set_prop(pc, "bLocked", Value::Bool(true));
        } else if target.is_none() && locked {
            let v = self.pv3(pc, "ViewAdjustAim");
            self.set_v3(pc, "MemAim", v);
            self.set_prop(pc, "bLocked", Value::Bool(false));
        }
        let locked = self.propr(pc, "bLocked").as_bool();
        let mut t = self.propr(pc, "fAdjustAimTrans").as_float();
        let mem = self.pv3(pc, "MemAim");
        if locked {
            t += step;
            if t >= 1.0 {
                t = 1.0;
            }
            self.set_prop(pc, "fAdjustAimTrans", Value::Float(t));
            if t == 1.0 {
                self.set_v3(pc, "ViewAdjustAim", mem);
                return r;
            }
            let v = [0, 1, 2].map(|k| (old[k] - mem[k]) * t + mem[k]);
            self.set_v3(pc, "ViewAdjustAim", v);
        } else {
            t -= step;
            if t <= 0.0 {
                t = 0.0;
            }
            self.set_prop(pc, "fAdjustAimTrans", Value::Float(t));
            if t == 0.0 {
                self.set_v3(pc, "ViewAdjustAim", [0.0; 3]);
                return r;
            }
            let v = [0, 1, 2].map(|k| (mem[k] - old[k]) * t + old[k]);
            self.set_v3(pc, "ViewAdjustAim", v);
        }
        r
    }

    /// Aim without a target: AdjustedAimForFiring follows the view, SmoothedAim(Rotation).
    fn aim_plain(&mut self, pc: ObjId, set_firing: bool) -> [i32; 3] {
        let rot = self.propr(pc, "Rotation").as_rotator();
        if set_firing {
            let d = crate::ai_move::rotator_vector(rot);
            self.set_v3(pc, "AdjustedAimForFiring", d);
        }
        self.smoothed_aim(pc, rot)
    }

    /// APlayerController::execAdjustAimForDisplay.
    pub(crate) fn adjust_aim_for_display(&mut self, pc: ObjId, ammo: Option<ObjId>, start: V) -> [i32; 3] {
        let alive = |vm: &Vm, o: Option<ObjId>| o.filter(|&x| vm.objects.get(x).is_some_and(|y| y.is_some()));
        let Some(ammo) = alive(self, ammo) else { return self.aim_plain(pc, true) };
        if self.propr(pc, "DefaultFOV").as_float() != self.propr(pc, "DesiredFOV").as_float() || self.propr(ammo, "bHandToHand").as_bool() {
            return self.aim_plain(pc, true);
        }
        let Some(pawn) = alive(self, self.propr(pc, "Pawn").as_object()) else { return self.aim_plain(pc, true) };
        let weapon = alive(self, self.propr(pawn, "Weapon").as_object());
        let trace_dist = weapon.map_or(0.0, |w| self.propr(w, "TraceDist").as_float());
        let rot = self.propr(pc, "Rotation").as_rotator();
        let dir = crate::ai_move::rotator_vector(rot);
        let trace_type = self.propr(ammo, "TraceType").as_int() as u32;
        let dist = if trace_dist <= 3000.0 { trace_dist } else { 3000.0 };
        let end = [dir[0] * dist + start[0], dir[1] * dist + start[1], dir[2] * dist + start[2]];
        let mut hit = CheckResult::new(1.0);
        self.single_line_check(&mut hit, Some(pawn), end, start, trace_type | 0x97, [0.0; 3]);
        // Aim point P0 and distance d of the kept target.
        // Co-op: a partner under the crosshair is not a target (the cone search below skips
        // partners too), so the view is never pulled towards the other player.
        let (target, d, p0) = match hit.actor.filter(|&a| self.propr(a, "bProjTarget").as_bool() && !self.coop_aim_partner(pawn, a)) {
            Some(a) => {
                if !self.is_a(a, "Pawn") || self.propr(a, "GameOver").as_byte() != 0 {
                    let r = self.smoothed_aim(pc, rot);
                    self.set_prop(pc, "BestTarget", Value::Object(Some(a)));
                    return r;
                }
                if self.propr(a, "bIsDead").as_bool() {
                    return self.smoothed_aim(pc, rot);
                }
                self.set_prop(pc, "BestTarget", Value::Object(Some(a)));
                let d = sub(self.pv3(a, "Location"), self.pv3(pawn, "Location"));
                // FVector::Size: (X² + Y²) + Z².
                let len = crate::ai_move::vsize(d);
                (a, len, hit.location)
            }
            None => {
                if self.propr(pc, "iAutoAimMode").as_int() == 0 {
                    return self.aim_plain(pc, true);
                }
                match self.auto_aim_target(pc, pawn, ammo, start, dir, trace_dist) {
                    None => {
                        self.set_prop(pc, "BestTarget", Value::Object(None));
                        return self.aim_plain(pc, true);
                    }
                    Some((t, d)) => {
                        self.set_prop(pc, "BestTarget", Value::Object(Some(t)));
                        if d > 3000.0 && d <= trace_dist {
                            return self.aim_plain(pc, true);
                        }
                        if d > trace_dist {
                            self.set_prop(pc, "BestTarget", Value::Object(None));
                            return self.aim_plain(pc, true);
                        }
                        (t, d, [dir[0] * d + start[0], dir[1] * d + start[1], dir[2] * d + start[2]])
                    }
                }
            }
        };
        let tloc = self.pv3(target, "Location");
        let to_target = safe_normal(sub(tloc, start));
        let c = [to_target[0] * d + start[0], to_target[1] * d + start[1], to_target[2] * d + start[2]];
        if !self.propr(ammo, "bInstantHit").as_bool() {
            // Projectile: target lead over d / Speed + 0.02 s (at most 2 s).
            let speed = self.propr(ammo, "ProjectileClass").as_class().map_or(0.0, |cl| {
                let cdo = self.classes[cl].cdo;
                self.propr(cdo, "Speed").as_float()
            });
            let mut t = d / speed + 0.02;
            if 2.0 <= t {
                t = 2.0;
            }
            let v = self.pv3(target, "Velocity");
            let lead = [t * v[0] + tloc[0], t * v[1] + tloc[1], t * v[2] + tloc[2]];
            let ld = safe_normal(sub(lead, start));
            let a = [d * ld[0] + start[0], d * ld[1] + start[1]];
            if (c[2] - p0[2]).abs() < self.propr(target, "CollisionHeight").as_float() {
                self.set_v3(pc, "AdjustedAimForFiring", [a[0] - start[0], a[1] - start[1], p0[2] - start[2]]);
                let r = vector_rotation([c[0] - start[0], c[1] - start[1], p0[2] - start[2]]);
                return self.smoothed_aim(pc, r);
            }
            self.set_prop(pc, "BestTarget", Value::Object(None));
            return self.aim_plain(pc, true);
        }
        let reach = self.propr(target, "CollisionHeight").as_float() + self.propr(target, "CollisionRadius").as_float();
        if !(reach > (c[2] - p0[2]).abs()) {
            self.set_prop(pc, "BestTarget", Value::Object(None));
            return self.smoothed_aim(pc, rot);
        }
        let old_best = self.propr(pc, "OldBestTarget").as_object();
        if self.propr(pc, "bLocked").as_bool() && old_best.is_some() && old_best != Some(target) {
            self.set_prop(pc, "bLocked", Value::Bool(false));
            self.set_prop(pc, "fAdjustAimTrans", Value::Float(0.0));
        }
        let r = vector_rotation([c[0] - start[0], c[1] - start[1], p0[2] - start[2]]);
        self.smoothed_aim(pc, r)
    }

    /// Co-op campaign (split screen, or hosting / joining one over the network; never a versus
    /// game): `o` is another player's pawn. The original has no co-op campaign; there its aim
    /// assist only ever meets the lone player's enemies, so a partner is left out of the aim
    /// targets as a teammate is in a team game. Never true in solo: solo aiming is unchanged.
    pub(crate) fn coop_aim_partner(&self, pawn: ObjId, o: ObjId) -> bool {
        let coop = self.coop.on && !self.coop.versus;
        #[cfg(test)]
        let coop = coop && !self.coop.aim_partners_kept;
        if !coop || o == pawn || !self.is_a(o, "Pawn") {
            return false;
        }
        // Only the players' pawns carry a PlayerReplicationInfo in the campaign (the story's
        // characters have none); a guest sees the host's pawn without its controller.
        self.propr(o, "PlayerReplicationInfo").as_object().is_some()
            || self.propr(o, "Controller").as_object().is_some_and(|c| self.objects.get(c).is_some_and(|x| x.is_some()) && self.is_a(c, "PlayerController"))
    }

    /// iAutoAimMode: up to five live pawns, drawn less than a second ago, in the
    /// aiming cone (cosine above a threshold depending on difficulty and ammunition type),
    /// sorted by cosine; the first one visible from the pawn's eyes is kept, with its distance.
    fn auto_aim_target(&mut self, pc: ObjId, pawn: ObjId, ammo: ObjId, start: V, dir: V, trace_dist: f32) -> Option<(ObjId, f32)> {
        let level = self.level?;
        let game = self.propr(level, "Game").as_object();
        let instant = self.propr(ammo, "bInstantHit").as_bool();
        let easy = game.is_some_and(|g| self.propr(g, "Difficulty").as_byte() == 0);
        let threshold = match (easy, instant) {
            (true, true) => 0.99,
            (true, false) => 0.96,
            (false, true) => 0.9925,
            (false, false) => 0.98,
        };
        // XLevel->TimeSeconds (double, +0xD0) − LastRenderTime by FSUBR qword, rounded once
        // to 24 bits by the FPU, then compared with 1.0.
        let now = self.level_time;
        let team_game = game.is_some_and(|g| self.propr(g, "bTeamGame").as_bool());
        let my_pri = self.propr(pc, "PlayerReplicationInfo").as_object();
        let mut dots = [-5000.0f32; 5];
        let mut actors = [None::<ObjId>; 5];
        let mut dists = [0.0f32; 5];
        let mut count = 0usize;
        let mut p = self.propr(level, "PawnList").as_object();
        let mut guard = 0;
        while let (Some(o), true) = (p, guard < 100_000) {
            guard += 1;
            let next = self.propr(o, "nextPawn").as_object();
            let ok = o != pawn
                && !self.coop_aim_partner(pawn, o)
                && self.propr(o, "Health").as_int() > 0
                && self.propr(o, "bProjTarget").as_bool()
                && ((now - self.propr(o, "LastRenderTime").as_float() as f64) as f32) < 1.0
                && self.propr(o, "GameOver").as_byte() == 0
                && !(team_game && {
                    let theirs = self.propr(o, "PlayerReplicationInfo").as_object();
                    match (my_pri, theirs) {
                        (Some(a), Some(b)) => self.propr(a, "Team").as_object() == self.propr(b, "Team").as_object(),
                        _ => false,
                    }
                });
            if ok {
                let d = sub(self.pv3(o, "Location"), start);
                let dot = d[1] * dir[1] + d[2] * dir[2] + d[0] * dir[0];
                if dot > 0.0 {
                    let sq = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                    let zoomed = self.is_a(pc, "PlayerController") && self.propr(pc, "FovAngle").as_float() != self.propr(pc, "DefaultFOV").as_float();
                    if sq < 9_000_000.0 || zoomed {
                        let dist = (sq as f64).sqrt() as f32;
                        if dist < trace_dist {
                            let cos = dot / dist;
                            if cos > threshold {
                                if let Some(i) = (0..5).find(|&i| cos > dots[i]) {
                                    for k in (i + 1..5).rev() {
                                        dots[k] = dots[k - 1];
                                        actors[k] = actors[k - 1];
                                        dists[k] = dists[k - 1];
                                    }
                                    dots[i] = cos;
                                    actors[i] = Some(o);
                                    dists[i] = dist;
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
            p = next;
        }
        let eye = {
            let l = self.pv3(pawn, "Location");
            [l[0], l[1], self.propr(pawn, "EyeHeight").as_float() + l[2]]
        };
        let flags = self.propr(ammo, "TraceType").as_int() as u32 | 0x286;
        for i in 0..count.min(5) {
            let Some(t) = actors[i] else { continue };
            let l = self.pv3(t, "Location");
            let target_eye = [l[0], l[1], self.propr(t, "EyeHeight").as_float() + l[2]];
            let mut hit = CheckResult::new(1.0);
            self.single_line_check(&mut hit, Some(pc), target_eye, eye, flags, [0.0; 3]);
            if hit.actor.is_none() {
                return Some((t, dists[i]));
            }
        }
        None
    }
}
