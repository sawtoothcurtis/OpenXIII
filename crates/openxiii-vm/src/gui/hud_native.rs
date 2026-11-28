//! XIII HUD natives reimplemented from Engine.dll: AHUD::execDrawPlayerInfo and
//! AHUD::execDrawStdBackground, which call UCanvas::DrawPlayerInfo
//! (vtable +0x7C) and UCanvas::DrawStdBackGround (vtable +0x78).
//!
//! HUD.DrawPlayerInfo(Canvas) draws the life block at the bottom left: one line per piece of information
//! (health, vest, helmet, breath, timer), each on a rounded background cut from the
//! HudIcons texture, with its icon and its text; then, if the sixth sense is active, two waves that
//! widen and the markers of the sounds heard.

use crate::value::{ObjId, Value};
use crate::Vm;

/// Truncation towards zero, like MSVC's _ftol.
fn ftol(x: f32) -> i32 {
    x as i32
}

/// Script Color (fields B, G, R, A) in FColor order.
fn bgra(v: &Value) -> [u8; 4] {
    match v {
        Value::Struct(_, f) if f.len() == 4 => [f[0].as_byte(), f[1].as_byte(), f[2].as_byte(), f[3].as_byte()],
        _ => [0; 4],
    }
}

fn color_value(c: [u8; 4]) -> Value {
    Value::Struct("object.color".into(), c.iter().map(|&b| Value::Byte(b)).collect())
}

/// Arguments of UCanvas::DrawPlayerInfo.
pub(crate) struct PlayerInfo {
    pub health: f32,
    pub vest: bool,
    pub helm: bool,
    pub breath: bool,
    pub breath_time: f32,
    pub chrono: bool,
    pub six_sense: bool,
    pub crouched: bool,
}

impl Vm {
    fn hud_f(&self, o: ObjId, name: &str) -> f32 {
        self.propr(o, name).as_float()
    }

    /// TimeSeconds (double) × k (qword constant), rounded only once to single precision
    /// like the 24-bit FPU: the exact product of two doubles (106 bits) is rounded to a
    /// float, which f64 then f32 does not do when the product falls halfway between two
    /// floats (the exact error of the double product, through FMA, breaks the tie).
    pub(crate) fn level_time_times(&self, k: f64) -> f32 {
        let (a, p) = (self.level_time, self.level_time * k);
        let r = p as f32;
        let (e, d) = (a.mul_add(k, -p), p - r as f64);
        if e == 0.0 || d == 0.0 || !r.is_finite() {
            return r;
        }
        let other = if d > 0.0 { r.next_up() } else { r.next_down() };
        if p == (r as f64 + other as f64) * 0.5 && (e > 0.0) == (d > 0.0) {
            return other;
        }
        r
    }

    /// Blinking warning colour: each component (alpha included) of HudWarningColor ×
    /// the fractional part of 1.5 × TimeSeconds.
    fn hud_blink(&self, hud: ObjId) -> [u8; 4] {
        let t = self.level_time_times(1.5);
        let frac = t - ftol(t) as f32;
        let w = bgra(&self.prop(hud, "HudWarningColor"));
        // Machine-code order: R, G, B, A.
        let r = ftol(w[2] as f32 * frac) as u8;
        let g = ftol(w[1] as f32 * frac) as u8;
        let b = ftol(w[0] as f32 * frac) as u8;
        let a = ftol(w[3] as f32 * frac) as u8;
        [b, g, r, a]
    }

    /// One text line of the life block (WrappedPrintf at ((H + XP) + 8, YP + 2)).
    fn hud_line_text(&mut self, hud: ObjId, canvas: ObjId, text: &str) {
        let h = self.propr(hud, "LifeDisplayHeight").as_int() as f32;
        let x = (h + self.hud_f(hud, "XP")) + 8.0;
        self.set_prop(canvas, "CurX", Value::Float(x));
        let y = self.hud_f(hud, "YP") + 2.0;
        self.set_prop(canvas, "CurY", Value::Float(y));
        let font = self.prop(canvas, "Font");
        self.canvas_wrapped_printf(canvas, &font, false, text);
    }

    /// YP -= LifeDisplayHeight.
    fn hud_next_line(&mut self, hud: ObjId) {
        let h = self.propr(hud, "LifeDisplayHeight").as_int() as f32;
        let yp = self.hud_f(hud, "YP") - h;
        self.set_prop(hud, "YP", Value::Float(yp));
    }

    /// AHUD::execDrawPlayerInfo: HUD.DrawPlayerInfo(Canvas C).
    pub(crate) fn hud_draw_player_info(&mut self, hud: ObjId, canvas: ObjId) {
        self.set_prop(canvas, "bUseBorder", Value::Bool(false));
        self.set_prop(canvas, "Style", Value::Byte(5));
        self.set_prop(canvas, "SpaceX", Value::Float(1.0));
        self.set_prop(canvas, "CurX", Value::Float(0.0));
        self.set_prop(canvas, "CurY", Value::Float(0.0));
        // Size of a line: "999" in the Canvas font (WrappedStrLenf).
        let font = self.prop(canvas, "Font");
        let (w, h) = self.canvas_wrapped_str_len(canvas, &font, "999 ");
        self.set_prop(hud, "LifeDisplayWidth", Value::Int(w));
        self.set_prop(hud, "LifeDisplayHeight", Value::Int(h));
        self.set_prop(hud, "IconSize", Value::Float((h as f32 + 4.0) * 0.03125));
        let Some(pawn) = self.propr(hud, "PawnOwner").as_object().filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some())) else {
            return;
        };
        let default_health = self.objects[pawn].as_ref().map(|o| self.propr(self.classes[o.class].cdo, "Health").as_int()).unwrap_or(0);
        let health = self.propr(pawn, "Health").as_int();
        let percent = if (health as f32) > 0.0 {
            let v = health as f32 / default_health as f32 * 100.0;
            ftol(if 1.0 < v { v } else { 1.0 })
        } else {
            0
        };
        let health_text = format!("{percent}");
        let armor = |vm: &Vm, n: &str| vm.propr(pawn, n).as_object().map(|a| format!("{}", vm.propr(a, "Charge").as_int()));
        let vest_text = armor(self, "Vest");
        let helm_text = armor(self, "Helm");
        let mut breath_time = 0.0f32;
        let breath_text = if self.propr(pawn, "bDrawBreathtimer").as_bool() {
            breath_time = self.hud_f(pawn, "DrownTimer") + 5.0;
            let m = ftol(breath_time * 0.016_666_668);
            let s = ftol((breath_time as f64 % 60.0) as f32);
            Some(if s < 10 { format!("{m}:0{s}") } else { format!("{m}:{s}") })
        } else {
            None
        };
        let y_start = ftol(self.hud_f(hud, "YP"));
        let info = PlayerInfo {
            health: percent as f32,
            vest: vest_text.is_some(),
            helm: helm_text.is_some(),
            breath: breath_text.is_some(),
            breath_time,
            chrono: self.propr(hud, "bDrawChrono").as_bool(),
            six_sense: self.propr(hud, "bDrawSixSense").as_bool(),
            crouched: self.propr(pawn, "bIsCrouched").as_bool(),
        };
        self.canvas_draw_player_info(canvas, hud, &info);
        self.set_prop(hud, "YP", Value::Float(y_start as f32 - h as f32));
        self.set_prop(canvas, "SpaceX", Value::Float(1.0));
        self.set_prop(canvas, "Style", Value::Byte(5));
        let basic = self.prop(hud, "HudBasicColor");
        let color = if percent < 30 { color_value(self.hud_blink(hud)) } else { basic.clone() };
        self.set_prop(canvas, "DrawColor", color);
        self.hud_line_text(hud, canvas, &health_text);
        for text in [vest_text, helm_text].into_iter().flatten() {
            self.hud_next_line(hud);
            self.set_prop(canvas, "DrawColor", basic.clone());
            self.hud_line_text(hud, canvas, &text);
        }
        if let Some(text) = breath_text {
            self.hud_next_line(hud);
            let color = if breath_time < 10.0 { color_value(self.hud_blink(hud)) } else { basic.clone() };
            self.set_prop(canvas, "DrawColor", color);
            self.hud_line_text(hud, canvas, &text);
        }
        if info.chrono {
            self.hud_next_line(hud);
            let color = if self.propr(hud, "bDrawChronoWithWarningColors").as_bool() { self.prop(hud, "HudWarningColor") } else { basic };
            self.set_prop(canvas, "DrawColor", color);
            let text = self.propr(hud, "sChronoString").as_name();
            self.hud_line_text(hud, canvas, &text);
        }
    }

    /// AHUD::execDrawStdBackground: SpaceX = 0, then UCanvas::DrawStdBackGround
    /// with HudWIcons.
    pub(crate) fn hud_draw_std_background(&mut self, hud: ObjId, canvas: ObjId, height: f32, center_width: f32) {
        self.set_prop(canvas, "SpaceX", Value::Float(0.0));
        let mat = self.prop(hud, "HudWIcons");
        self.canvas_draw_std_background(canvas, &mat, center_width, height);
    }

    fn obj_live(&self, v: Value) -> Option<ObjId> {
        v.as_object().filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some()))
    }

    /// AWeapon::HasAmmo.
    pub(crate) fn weapon_has_ammo(&self, w: ObjId) -> bool {
        let Some(ammo) = self.obj_live(self.prop(w, "AmmoType")) else { return false };
        let amount = self.propr(ammo, "AmmoAmount").as_int();
        let reload = |n: &str| self.obj_live(self.prop(w, n)).map_or(0, |s| self.propr(s, "ReloadCount").as_byte() as i32);
        if self.propr(w, "bHaveSlave").as_bool() {
            return amount - reload("MySlave") > 0;
        }
        if self.propr(w, "bIsSlave").as_bool() {
            return amount - reload("SlaveOf") > 0;
        }
        amount > 0
    }

    /// AWeapon::HasAltAmmo.
    pub(crate) fn weapon_has_alt_ammo(&self, w: ObjId) -> bool {
        self.obj_live(self.prop(w, "AltAmmoType")).is_some_and(|a| self.propr(a, "AmmoAmount").as_int() > 0)
    }

    /// ReloadCount from the default values of the weapon's class.
    fn class_reload_count(&self, w: ObjId) -> f32 {
        self.objects[w].as_ref().map_or(0, |o| self.propr(self.classes[o.class].cdo, "ReloadCount").as_byte()) as f32
    }

    /// AHUD::execDrawAmmo: HUD.DrawAmmo(Canvas C, string ItemText) — background (DrawStdBackGround
    /// of HudWIcons), BulletIconTex icon and ammunition text, at the bottom right.
    pub(crate) fn hud_draw_ammo(&mut self, hud: ObjId, canvas: ObjId, text: &str) {
        // Co-op: a player who is down watches their body (its weapon is still the HUD's
        // DrawnWeapon) until they come back: no ammo counter during that wait.
        if self.coop_owner_down(hud) {
            return;
        }
        let saved = self.prop(canvas, "DrawColor");
        let h = self.propr(hud, "LifeDisplayHeight").as_int();
        let yp = self.hud_f(hud, "YP") - h as f32;
        self.set_prop(hud, "YP", Value::Float(yp));
        self.set_prop(canvas, "SpaceX", Value::Float(1.0));
        self.set_prop(canvas, "CurX", Value::Float(0.0));
        self.set_prop(canvas, "CurY", Value::Float(0.0));
        let font = self.prop(canvas, "Font");
        let (xl, _) = self.canvas_wrapped_str_len(canvas, &font, text);
        self.set_prop(canvas, "Style", Value::Byte(5));
        let bg = self.prop(hud, "HudBackGroundColor");
        self.set_prop(canvas, "DrawColor", bg);
        let x = (self.hud_f(hud, "XP") - (2 * h - 8) as f32) - (xl + 8) as f32;
        self.set_prop(canvas, "CurX", Value::Float(x));
        let y = self.hud_f(hud, "YP") + 4.0;
        self.set_prop(canvas, "CurY", Value::Float(y));
        let wicons = self.prop(hud, "HudWIcons");
        self.canvas_draw_std_background(canvas, &wicons, (xl + 16) as f32, (h - 4) as f32);
        let white = self.prop(hud, "WhiteColor");
        self.set_prop(canvas, "DrawColor", white.clone());
        let yp = self.hud_f(hud, "YP");
        self.set_prop(canvas, "CurY", Value::Float(yp));
        let tex = self.prop(hud, "BulletIconTex");
        let (us, vs) = self.material_size(&tex);
        let icon = self.hud_f(hud, "IconSize");
        let (cx, ox) = (self.hud_f(canvas, "CurX"), self.hud_f(canvas, "OrgX"));
        let (cy, oy) = (self.hud_f(canvas, "CurY"), self.hud_f(canvas, "OrgY"));
        let z = self.hud_f(canvas, "Z");
        let color = crate::canvas::fplane(bgra(&white));
        self.canvas_draw_tile(canvas, &tex, cx + ox, cy + oy, us as f32 * icon, vs as f32 * icon, 0.0, 0.0, us as f32, vs as f32, z, color, [0.0; 4]);
        self.set_prop(canvas, "bTextShadow", Value::Bool(true));
        let x = (((self.hud_f(hud, "XP") - (2 * h - 8) as f32) - (xl + 8) as f32) + h as f32) + 8.0;
        self.set_prop(canvas, "CurX", Value::Float(x));
        let y = self.hud_f(hud, "YP") + 2.0;
        self.set_prop(canvas, "CurY", Value::Float(y));
        self.set_prop(canvas, "DrawColor", saved);
        self.canvas_wrapped_printf(canvas, &font, false, text);
        self.set_prop(canvas, "bTextShadow", Value::Bool(false));
    }

    /// AHUD::execDrawWeaponsList: HUD.DrawWeaponsList(Canvas C) — weapons of the
    /// inventory sorted by InventoryGroup, UCanvas::DrawWeaponList then the remaining bullets
    /// (sAmmoRef, at most 6) under each weapon.
    pub(crate) fn hud_draw_weapons_list(&mut self, hud: ObjId, canvas: ObjId) {
        let mut list: [Option<ObjId>; 25] = [None; 25];
        let mut count = 0i32;
        let mut max_group = 0usize;
        if let Some(pawn) = self.obj_live(self.prop(hud, "PawnOwner")) {
            let mut inv = self.obj_live(self.prop(pawn, "Inventory"));
            while let Some(w) = inv {
                if self.is_a(w, "Weapon") && self.obj_live(self.prop(w, "AmmoType")).is_some() {
                    let keep = self.class_reload_count(w) != 0.0 || self.weapon_has_ammo(w);
                    if keep && self.propr(w, "WHand").as_byte() != 4 && !self.propr(w, "bIsSlave").as_bool() {
                        let g = self.propr(w, "InventoryGroup").as_byte() as usize;
                        if g < list.len() {
                            list[g] = Some(w);
                        }
                        max_group = max_group.max(g);
                        count += 1;
                    }
                }
                inv = self.obj_live(self.prop(w, "Inventory"));
            }
        }
        let h = self.propr(hud, "LifeDisplayHeight").as_int();
        let yp = (1.0 - self.hud_f(hud, "DownMargin")) * self.hud_f(canvas, "ClipY") - h as f32;
        self.set_prop(hud, "YP", Value::Float(yp));
        self.set_prop(canvas, "Style", Value::Byte(5));
        self.canvas_draw_weapon_list(canvas, hud, &list, count, max_group);
        let mut x = ftol(((1.0 - self.hud_f(hud, "RightMargin")) * self.hud_f(canvas, "ClipX") - (((h + 2) * count) << 1) as f32) - (h / 2) as f32);
        let drawn = self.propr(hud, "DrawnWeapon").as_object();
        for &slot in list.iter().take(max_group + 1) {
            let Some(w) = slot else { continue };
            let mut basic = bgra(&self.prop(hud, "HudBasicColor"));
            basic[3] = if Some(w) == drawn { 0xff } else { 0x96 };
            self.set_prop(canvas, "DrawColor", color_value(basic));
            let reload = self.class_reload_count(w);
            let amount = self.obj_live(self.prop(w, "AmmoType")).map_or(0, |a| self.propr(a, "AmmoAmount").as_int());
            let n = if reload > 0.0 {
                Some(ftol(amount as f32 / reload))
            } else if self.propr(w, "WHand").as_byte() != 0 {
                Some(amount)
            } else {
                None
            };
            if let Some(n) = n.filter(|&n| n > 0) {
                let n = n.min(6) as usize;
                let text: String = self.propr(hud, "sAmmoRef").as_name().chars().take(n).collect();
                self.set_prop(canvas, "CurX", Value::Float(x as f32));
                let y = (self.hud_f(hud, "YP") - (h / 2) as f32) - 6.0;
                self.set_prop(canvas, "CurY", Value::Float(y));
                let font = self.prop(canvas, "Font");
                self.canvas_wrapped_printf(canvas, &font, false, &text);
            }
            x = x + 2 * h + 4;
        }
    }
}
