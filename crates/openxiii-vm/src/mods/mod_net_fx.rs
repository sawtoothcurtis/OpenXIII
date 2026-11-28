//! Mods build only (feature `mods`): what the host's scripts make appear, shown on network guests.
//!
//! The scripts run on the host only. What they make with `effect{}` (emitters, marks, silent
//! sound carriers) is given a role on the network (RemoteRole SimulatedProxy, always relevant):
//! the game's own replication then creates each one on every guest, with its class (the mods'
//! classes are in the package map, mod_net.rs), and moves it (Location, Rotation, Base). What
//! that replication does not carry travels in the text of reliable `ClientMessage` calls (an
//! RPC every PlayerController has), as for the mods' skins (mod_net_skins.rs):
//!
//! - what a script does to such an actor afterwards: a property written (`Emitters[0].Disabled`,
//!   `LifeSpan`, a sound's reach), a function called (`Trigger`, `AttachProjector`), a sound
//!   played from it. Each actor keeps the list of these steps; a guest is sent the steps it has
//!   not had yet once the actor's channel is open for it, so a guest that joins late gets the
//!   whole list (a fire that was put out stays out, a mark is attached again);
//! - what a script changes in the level: how charred an actor is drawn (`charred`), the level's
//!   charred areas (`char_area`), textures changed on scenery (`retexture`), heat shimmer
//!   (`heat{}`). The charred values and areas and the textures are kept as they are now and sent
//!   in full to a guest that joins.
//!
//! - where a script moves such an actor (`SetLocation`, `SetRotation`): the game does not send
//!   an emitter's place after it was made, nor ever its rotation. The place goes as a step
//!   that is only kept as it is now, at most ten times a second and only when it changed:
//!   relative to its owner's view when only its owner sees it (a gun's flame seen in first
//!   person: placed again on the guest every frame at its own view, without delay), at a point
//!   of another actor when a script pinned it there (`pin_to`: placed every frame on that
//!   actor as the guest draws it, a gun in a character's hands posed as that character is,
//!   turned with that character's view), relative to its owner when that is a character
//!   (placed every frame at where that character is), else where it is;
//!
//! Actors are named by their channel on that guest's connection, or by their name for the
//! level's own actors (the same map on both sides). Nothing of this is noted, sent or read
//! without a script using these functions in a network game, and nothing is compiled without the
//! `mods` feature: the game's network protocol is unchanged then.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::collections::HashMap;

use crate::value::{ObjId, Value};
use crate::Vm;

/// Start of the text of an effects message.
pub const FX_MESSAGE: &str = "OXIIIMod.Fx|";
/// Between the steps of a message, and between the fields of a step (characters no name,
/// property path or number uses).
const STEP_SEP: char = '~';
const FIELD_SEP: char = '`';
/// A message's text is kept under this many bytes as the network writes it (several messages a
/// tick if needed): a ClientMessage call must fit in one packet (512 bytes with its headers),
/// or the game drops it ("RPC bunch overflowed").
const MESSAGE_MAX: usize = 380;

/// Bytes the network writes for `text`: one per character, two for all of them when one is
/// beyond Latin-1 (the game's strings then go as UTF-16).
fn wire_len(text: &str) -> usize {
    let n = text.chars().count();
    if text.chars().any(|c| c as u32 > 255) { 2 * n } else { n }
}

/// The host's record, and the guest's steps waiting for their actor.
#[derive(Default)]
pub struct NetFx {
    /// Host: by effect actor, the steps done to it (property writes, calls, sounds).
    pub steps: HashMap<ObjId, Vec<String>>,
    /// Host: by (connection, actor), how many of its steps that guest has had.
    pub sent: HashMap<(usize, ObjId), usize>,
    /// Host: charred actors last sent to each connection (connection, actor) → value.
    pub charred_sent: HashMap<(usize, ObjId), f32>,
    /// Host: steps for the level (areas, textures), and how many each connection has had.
    pub level: Vec<String>,
    pub level_sent: HashMap<usize, usize>,
    /// Host: heat zones made since the last tick, sent once to the guests there.
    pub heat_new: Vec<String>,
    /// Host: by name, when a named heat zone (made again every frame by a script) was last
    /// sent.
    pub heat_sent: HashMap<String, f64>,
    /// Guest: steps for channels whose actor has not arrived yet.
    pub pending: HashMap<usize, Vec<Vec<String>>>,
    /// Guest: level steps naming a channel whose actor has not arrived yet.
    pub pending_level: Vec<Vec<String>>,
    /// Host: effect actors a script moved since the last tick.
    pub moved: std::collections::HashSet<ObjId>,
    /// Host: by (connection, actor), the place last sent and when.
    pub place_sent: HashMap<(usize, ObjId), (Place, f64)>,
    /// Guest: effect actors kept at a place relative to a character, by actor.
    pub follow: HashMap<ObjId, Place>,
    /// Host and game alone: actors a script pinned to a point of another actor (`pin_to`), by
    /// actor: that actor and the point in its frame (forward, right, up). Placed every tick.
    pub pins: HashMap<ObjId, (ObjId, [f32; 3])>,
    /// Host: by (connection, area), the charred amount last sent of the level's charred areas
    /// (mod_retexture::char_areas, which only grows during a level).
    pub areas_sent: HashMap<(usize, usize), f32>,
    /// Host: what was sent since `counted_from` (messages, characters), written in the mods'
    /// log once a minute.
    pub sent_messages: usize,
    pub sent_bytes: usize,
    /// Host: by kind of step (m place, s property, c call, p sound, ch charred, ar area...),
    /// how many and how many characters, for the same line.
    pub sent_kinds: std::collections::BTreeMap<String, (usize, usize)>,
    pub counted_from: Option<f64>,
}

/// Where an effect actor is: relative to its owner's view ('v'), to its owner ('o'), at a point
/// of another actor ('b', `pin_to`: `to` is that actor, `at` the point in its frame, the
/// rotation relative to the view of the character that actor is on), or in the world ('a');
/// `to` is the owner for 'v' and 'o'. Location in the view's axes (forward, right, up) for 'v'.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub how: u8,
    pub to: Option<ObjId>,
    pub at: [f32; 3],
    pub rot: [i32; 3],
}

impl Place {
    /// Close enough to `o` that it need not be sent again: 3 units and about 1.5 degrees at a
    /// view (what the player sees in front of him), 8 units and about 3 degrees at a
    /// character, 16 units in the world (a sound carrier, a mark).
    fn near(&self, o: &Place) -> bool {
        let d = (0..3).map(|k| (self.at[k] - o.at[k]).powi(2)).sum::<f32>().sqrt();
        let r = (0..3).map(|k| ((self.rot[k] - o.rot[k]) as i16 as i32).abs()).max().unwrap_or(0);
        let (dd, rr) = match self.how {
            b'v' | b'b' => (3.0, 280),
            b'o' => (8.0, 550),
            _ => (16.0, 550),
        };
        self.how == o.how && self.to == o.to && d < dd && r < rr
    }

    /// Seconds between two places sent for one actor: 0.1 at a view or pinned, 0.2 otherwise.
    fn pace(&self) -> f64 {
        if matches!(self.how, b'v' | b'b') { 0.1 } else { 0.2 }
    }
}

/// Axes of a view rotation (pitch, yaw): forward, right, up, as the scripts reckon them.
fn view_axes(rot: [i32; 3]) -> [[f32; 3]; 3] {
    let (p, y) = (rot[0] as f32 * std::f32::consts::PI / 32768.0, rot[1] as f32 * std::f32::consts::PI / 32768.0);
    let (cp, sp, cy, sy) = (p.dcos(), p.dsin(), y.dcos(), y.dsin());
    [[cp * cy, cp * sy, sp], [-sy, cy, 0.0], [-sp * cy, -sp * sy, cp]]
}

/// A property's value as text ("b1", "f0.5", "i3", "s…", "n…"), None for what is not sent.
fn encode(v: &Value) -> Option<String> {
    Some(match v {
        Value::Bool(b) => format!("b{}", *b as u8),
        Value::Byte(x) => format!("y{x}"),
        Value::Int(x) => format!("i{x}"),
        Value::Float(x) => format!("f{x}"),
        Value::Str(s) => format!("s{s}"),
        Value::Name(n) => format!("n{n}"),
        Value::Asset(a) => format!("a{a}"),
        _ => return None,
    })
}

fn decode(t: &str) -> Option<Value> {
    let (k, rest) = t.split_at(t.char_indices().nth(1).map_or(t.len(), |(i, _)| i));
    Some(match k {
        "b" => Value::Bool(rest == "1"),
        "y" => Value::Byte(rest.parse().ok()?),
        "i" => Value::Int(rest.parse().ok()?),
        "f" => Value::Float(rest.parse().ok()?),
        "s" => Value::Str(rest.to_string()),
        "n" => Value::Name(rest.to_string().into()),
        "a" => Value::Asset(rest.to_string()),
        _ => return None,
    })
}

/// The steps of a message, each split in its fields; None for any other text.
pub fn parse_fx_message(text: &str) -> Option<Vec<Vec<String>>> {
    let rest = text.strip_prefix(FX_MESSAGE)?;
    Some(rest.split(STEP_SEP).filter(|s| !s.is_empty()).map(|s| s.split(FIELD_SEP).map(str::to_string).collect()).collect())
}

fn step(fields: &[&str]) -> String {
    fields.join(&FIELD_SEP.to_string())
}

impl Vm {
    /// A network host (a listen server): its scripts' effects are shared with its guests.
    pub(crate) fn mod_net_host(&self) -> bool {
        self.ipdrv.net_driver.as_ref().is_some_and(|d| d.server_conn.is_none())
    }

    /// Host: an actor `effect{}` made is replicated to the guests (the game creates and moves
    /// it there), and what is done to it afterwards is noted.
    pub(crate) fn mod_net_fx_spawned(&mut self, a: ObjId) {
        if !self.mod_net_host() {
            return;
        }
        self.set_prop(a, "RemoteRole", Value::Byte(2));
        self.set_prop(a, "bAlwaysRelevant", Value::Bool(true));
        // A new actor can take the number of one taken out of the game in the same tick (a
        // script making its effects again): nothing of the old one's record may count for it, or
        // its first steps would be taken as already sent.
        self.mod_net_fx_forget(a);
        self.mods.net_fx.steps.insert(a, Vec::new());
    }

    /// Host: what was noted and sent about actor number `a` is forgotten.
    fn mod_net_fx_forget(&mut self, a: ObjId) {
        let fx = &mut self.mods.net_fx;
        fx.steps.remove(&a);
        fx.sent.retain(|(_, x), _| *x != a);
        fx.place_sent.retain(|(_, x), _| *x != a);
        fx.charred_sent.retain(|(_, x), _| *x != a);
        fx.moved.remove(&a);
        fx.pins.remove(&a);
    }

    /// Host: property `key` of effect actor `a` was written (its value as it is now).
    pub(crate) fn mod_net_fx_set(&mut self, a: ObjId, key: &str) {
        if !self.mods.net_fx.steps.contains_key(&a) {
            return;
        }
        let Ok((o, last)) = crate::mod_script_world::property_path(self, a, key) else { return };
        let Some(text) = self.prop_slot(o, &last).ok().and_then(|_| encode(&self.prop(o, &last))) else { return };
        let s = step(&["s", key, &text]);
        let Some(list) = self.mods.net_fx.steps.get_mut(&a) else { return };
        // A property written again: nothing when its value did not change (scripts often set
        // the same value every frame); else its earlier value goes when no call or sound came
        // after it, so that the list stays as short as what it does.
        let prefix = format!("s{FIELD_SEP}{key}{FIELD_SEP}");
        if let Some(k) = list.iter().rposition(|x| x.starts_with(&prefix)) {
            if list[k] == s {
                return;
            }
            if !list[k + 1..].iter().any(|x| !x.starts_with('s')) {
                list.remove(k);
                // What each guest has had moves back by one past that step: the new value is
                // then sent to those who had the old one.
                for ((_, x), n) in self.mods.net_fx.sent.iter_mut() {
                    if *x == a && *n > k {
                        *n -= 1;
                    }
                }
            }
        }
        if let Some(list) = self.mods.net_fx.steps.get_mut(&a) {
            list.push(s);
        }
    }

    /// Host: function `name` of effect actor `a` was called (movement is replicated: not noted).
    pub(crate) fn mod_net_fx_call(&mut self, a: ObjId, name: &str) {
        if !self.mods.net_fx.steps.contains_key(&a) {
            return;
        }
        if name.eq_ignore_ascii_case("SetLocation") || name.eq_ignore_ascii_case("SetRotation") {
            self.mods.net_fx.moved.insert(a);
            return;
        }
        // The owner and the base travel with the game's replication (and an actor given as an
        // argument could not be named here).
        if name.eq_ignore_ascii_case("SetOwner") || name.eq_ignore_ascii_case("SetBase") {
            return;
        }
        // A function that gives back a value is a question (IsA, a test, a search): only what
        // acts is sent.
        if self.find_function(a, name, false).is_some_and(|f| self.functions[f].ret.is_some()) {
            return;
        }
        if let Some(list) = self.mods.net_fx.steps.get_mut(&a) {
            list.push(step(&["c", name]));
        }
    }

    /// Host: sound `name` played from effect actor `a`. False when `a` is not an effect.
    pub(crate) fn mod_net_fx_sound(&mut self, a: ObjId, name: &str) -> bool {
        match self.mods.net_fx.steps.get_mut(&a) {
            Some(list) => {
                list.push(step(&["p", name]));
                true
            }
            None => false,
        }
    }

    /// Host: a level change (a charred area, a texture) to send to every guest, now and later.
    pub(crate) fn mod_net_level_step(&mut self, fields: &[&str]) {
        if self.mod_net_host() {
            self.mods.net_fx.level.push(step(fields));
        }
    }

    /// Host: heat shimmer to show on the guests there now.
    pub(crate) fn mod_net_heat(&mut self, z: &crate::mod_heat::HeatZone, seconds: f32) {
        if !self.mod_net_host() {
            return;
        }
        // A named zone a script makes again every frame (over a jet, a burning man): at most
        // four times a second, each lasting until the next.
        let mut seconds = seconds;
        if !z.name.is_empty() {
            let now = self.mod_level_time() as f64;
            if self.mods.net_fx.heat_sent.get(&z.name).is_some_and(|&t| now - t < 0.25) {
                return;
            }
            self.mods.net_fx.heat_sent.insert(z.name.clone(), now);
            seconds = seconds.max(0.35);
        }
        let on = z.on.map(|a| format!("{a}")).unwrap_or_default();
        let at = format!("{:.0},{:.0},{:.0}", z.at[0], z.at[1], z.at[2]);
        self.mods.net_fx.heat_new.push(step(&["h", &on, &at, &format!("{:.0}", z.radius), &format!("{seconds:.2}"), &format!("{:.2}", z.strength), &z.name]));
    }

    /// How connection `i` names actor `a`: "c<channel>" when its channel is open there, "n<name>"
    /// for the level's own actors, None otherwise (not there yet).
    fn mod_net_ref(&self, i: usize, a: ObjId) -> Option<String> {
        let conn = self.net_conn_ref(Some(i))?;
        if let Some(ch) = conn.actor_map.get(&a) {
            return Some(format!("c{ch}"));
        }
        let o = self.objects.get(a)?.as_ref()?;
        // The map's actors (loaded from the map on both sides) keep their names.
        if self.propr(a, "bStatic").as_bool() || self.propr(a, "bNoDelete").as_bool() || self.propr(a, "RemoteRole").as_byte() == 0 {
            return Some(format!("n{}", o.name));
        }
        None
    }

    /// A character's view: its eyes and its controller's rotation; without a controller here (a
    /// player of another machine, on a guest), its own turn and the pitch the host sends with
    /// its body (ControllerPitch, as coop_look.rs reads it).
    fn mod_net_view(&self, pawn: ObjId) -> ([f32; 3], [i32; 3]) {
        let l = self.prop(pawn, "Location").as_vector();
        let eyes = [l[0], l[1], l[2] + self.prop(pawn, "EyeHeight").as_float()];
        let c = self.prop(pawn, "Controller").as_object().filter(|&c| self.objects.get(c).is_some_and(|o| o.is_some()));
        let rot = match c {
            Some(c) => self.prop(c, "Rotation").as_rotator(),
            None => {
                let r = self.prop(pawn, "Rotation").as_rotator();
                match self.prop(pawn, "ControllerPitch") {
                    Value::Byte(b) => [((b as i32) << 8) as u16 as i16 as i32, r[1], 0],
                    _ => r,
                }
            }
        };
        (eyes, rot)
    }

    /// The character actor `t` is on: itself, or its base (a gun held in a hand).
    fn mod_pin_character(&self, t: ObjId) -> Option<ObjId> {
        let alive = |x: ObjId| self.objects.get(x).is_some_and(|o| o.is_some());
        if self.is_a(t, "Pawn") {
            return Some(t);
        }
        self.prop(t, "Base").as_object().filter(|&b| alive(b) && self.is_a(b, "Pawn"))
    }

    /// Point `at` (forward, right, up) of actor `t`'s frame as `t` is now. An actor held at a
    /// bone of a character (a gun in a hand) is first put where that bone is in the character's
    /// latest pose (USkeletalMesh::SetAttachmentLocation, as the render does): the point is on
    /// the gun as this machine draws it, even when the character was not drawn this frame.
    pub(crate) fn mod_pin_point(&mut self, t: ObjId, at: [f32; 3]) -> [f32; 3] {
        if !self.propr(t, "AttachmentBone").as_name().is_empty() {
            if let Some(base) = self.mod_pin_character(t).filter(|&b| b != t) {
                if self.native_prop(base, crate::NativeField::DrawType).as_byte() == 2 && self.mesh_instance(base).is_some() {
                    let _ = self.set_attachment_location(base, t);
                }
            }
        }
        let l = self.prop(t, "Location").as_vector();
        let ax = crate::ai_natives::axes(self.prop(t, "Rotation").as_rotator());
        [0, 1, 2].map(|k| l[k] + ax[0][k] * at[0] + ax[1][k] * at[1] + ax[2][k] * at[2])
    }

    /// Host and game alone, every tick after the scripts: the pinned actors go to their point.
    pub(crate) fn mod_pins_apply(&mut self) {
        if self.mods.net_fx.pins.is_empty() {
            return;
        }
        let list: Vec<(ObjId, (ObjId, [f32; 3]))> = self.mods.net_fx.pins.iter().map(|(&a, &p)| (a, p)).collect();
        for (a, (t, at)) in list {
            let alive = |x: ObjId| self.objects.get(x).is_some_and(|o| o.is_some());
            if !alive(a) || !alive(t) {
                self.mods.net_fx.pins.remove(&a);
                continue;
            }
            let p = self.mod_pin_point(t, at);
            let mut args = [Value::vector(p[0], p[1], p[2])];
            let _ = crate::natives::call(self, "SetLocation", a, &mut args, &[true]);
        }
    }

    /// `pin_to`: actor `a` kept at point `at` of `t` (None: let go). Its place is sent again.
    pub(crate) fn mod_pin(&mut self, a: ObjId, to: Option<(ObjId, [f32; 3])>) {
        match to {
            Some(p) => {
                self.mods.net_fx.pins.insert(a, p);
            }
            None => {
                self.mods.net_fx.pins.remove(&a);
            }
        }
        if self.mods.net_fx.steps.contains_key(&a) {
            self.mods.net_fx.moved.insert(a);
        }
        self.mod_pins_apply();
    }

    /// Host: where effect actor `a` is, as it is sent.
    fn mod_net_place(&self, a: ObjId) -> Place {
        let at = self.prop(a, "Location").as_vector();
        let rot = self.prop(a, "Rotation").as_rotator();
        if let Some(&(t, off)) = self.mods.net_fx.pins.get(&a) {
            let rel = match self.mod_pin_character(t) {
                Some(c) => {
                    let vr = self.mod_net_view(c).1;
                    [0, 1, 2].map(|k| (rot[k] - vr[k]) as i16 as i32)
                }
                None => rot,
            };
            return Place { how: b'b', to: Some(t), at: off, rot: rel };
        }
        let owner = self.prop(a, "Owner").as_object().filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some()) && self.is_a(o, "Pawn"));
        match owner {
            Some(o) if self.propr(a, "bOnlyOwnerSee").as_bool() => {
                let (eyes, vr) = self.mod_net_view(o);
                let ax = view_axes(vr);
                let d = [at[0] - eyes[0], at[1] - eyes[1], at[2] - eyes[2]];
                let rel = [0, 1, 2].map(|k| d[0] * ax[k][0] + d[1] * ax[k][1] + d[2] * ax[k][2]);
                Place { how: b'v', to: Some(o), at: rel, rot: [0, 1, 2].map(|k| (rot[k] - vr[k]) as i16 as i32) }
            }
            Some(o) => {
                let l = self.prop(o, "Location").as_vector();
                Place { how: b'o', to: Some(o), at: [at[0] - l[0], at[1] - l[1], at[2] - l[2]], rot }
            }
            None => Place { how: b'a', to: None, at, rot },
        }
    }

    /// Host, after the replication of a tick: each guest is sent what it has not had yet.
    pub(crate) fn mod_net_send_fx(&mut self) {
        // No mod script running here: nothing of this is ever sent (the game's network unchanged).
        if self.mods.script_sources.is_none() {
            return;
        }
        if self.mods.net_fx.steps.is_empty()
            && self.mods.net_fx.level.is_empty()
            && self.mod_charred.is_empty()
            && self.mods.net_fx.heat_new.is_empty()
            && self.mods.surfaces.char_areas_generation() == 0
        {
            return;
        }
        let now = self.mod_level_time() as f64;
        let moved: Vec<ObjId> = self.mods.net_fx.moved.iter().copied().filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some())).collect();
        let places: Vec<(ObjId, Place)> = moved.iter().map(|&a| (a, self.mod_net_place(a))).collect();
        // Actors gone: their record goes too.
        let gone: Vec<ObjId> = self.mods.net_fx.steps.keys().copied().filter(|&a| self.objects.get(a).is_none_or(|o| o.is_none())).collect();
        for a in gone {
            self.mods.net_fx.steps.remove(&a);
            self.mods.net_fx.sent.retain(|(_, x), _| *x != a);
            self.mods.net_fx.place_sent.retain(|(_, x), _| *x != a);
            self.mods.net_fx.moved.remove(&a);
        }
        let mut still_moving = std::collections::HashSet::new();
        let heat = std::mem::take(&mut self.mods.net_fx.heat_new);
        let n = self.ipdrv.net_driver.as_ref().map_or(0, |d| d.conns.len());
        for i in 0..n {
            let Some(pc) = self.net_conn_ref(Some(i)).and_then(|c| c.pc).filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some())) else { continue };
            let mut out: Vec<String> = Vec::new();
            // Effects whose channel is open on this connection.
            let actors: Vec<ObjId> = self.mods.net_fx.steps.keys().copied().collect();
            for a in actors {
                let Some(ch) = self.net_conn_ref(Some(i)).and_then(|c| c.actor_map.get(&a)).copied() else { continue };
                let done = self.mods.net_fx.sent.get(&(i, a)).copied().unwrap_or(0);
                let list = &self.mods.net_fx.steps[&a];
                if done < list.len() {
                    for s in &list[done..] {
                        out.push(format!("e{ch}{FIELD_SEP}{s}"));
                    }
                    let len = list.len();
                    self.mods.net_fx.sent.insert((i, a), len);
                }
            }
            // Places of the actors moved (at most ten times a second each, when they changed).
            for &(a, place) in &places {
                let Some(ch) = self.net_conn_ref(Some(i)).and_then(|c| c.actor_map.get(&a)).copied() else {
                    still_moving.insert(a);
                    continue;
                };
                let last = self.mods.net_fx.place_sent.get(&(i, a)).copied();
                if last.is_some_and(|(p, _)| p.near(&place)) {
                    continue;
                }
                if last.is_some_and(|(_, t)| now - t < place.pace()) {
                    still_moving.insert(a);
                    continue;
                }
                let to = match place.to {
                    Some(o) => match self.mod_net_ref(i, o) {
                        Some(r) => r,
                        None => {
                            still_moving.insert(a);
                            continue;
                        }
                    },
                    None => String::new(),
                };
                let at = format!("{:.1},{:.1},{:.1}", place.at[0], place.at[1], place.at[2]);
                let rot = format!("{},{},{}", place.rot[0], place.rot[1], place.rot[2]);
                out.push(format!("e{ch}{FIELD_SEP}{}", step(&["m", &(place.how as char).to_string(), &to, &at, &rot])));
                self.mods.net_fx.place_sent.insert((i, a), (place, now));
            }
            // Charred actors (their latest value).
            let charred: Vec<(ObjId, f32)> = self.mod_charred.iter().map(|(&a, &k)| (a, k)).collect();
            for (a, k) in charred {
                // By steps of a tenth (charring goes on for many seconds), the end exactly.
                if self.mods.net_fx.charred_sent.get(&(i, a)).is_some_and(|&s| (s - k).abs() < 0.1 && !((k >= 0.999 || k <= 0.001) && s != k)) {
                    continue;
                }
                if let Some(r) = self.mod_net_ref(i, a) {
                    out.push(step(&["w", "ch", &r, &format!("{k:.3}")]));
                    self.mods.net_fx.charred_sent.insert((i, a), k);
                }
            }
            // The level's charred areas, as they are now (by steps of a tenth, the end exactly).
            for (n, (at, radius, k)) in self.mods.surfaces.char_areas().into_iter().enumerate() {
                let last = self.mods.net_fx.areas_sent.get(&(i, n)).copied();
                if last.is_some_and(|s| (k - s) < 0.1 && !(k >= 0.999 && s < 0.999)) {
                    continue;
                }
                out.push(step(&["w", "ar", &format!("{:.0}", at[0]), &format!("{:.0}", at[1]), &format!("{:.0}", at[2]), &format!("{radius:.0}"), &format!("{k:.2}")]));
                self.mods.net_fx.areas_sent.insert((i, n), k);
            }
            // The level's steps not had yet.
            let done = self.mods.net_fx.level_sent.get(&i).copied().unwrap_or(0);
            if done < self.mods.net_fx.level.len() {
                out.extend(self.mods.net_fx.level[done..].iter().map(|s| format!("w{FIELD_SEP}{s}")));
                let len = self.mods.net_fx.level.len();
                self.mods.net_fx.level_sent.insert(i, len);
            }
            // Heat shimmer made this tick, its actor named for this guest.
            for h in &heat {
                let mut f: Vec<String> = h.split(FIELD_SEP).map(str::to_string).collect();
                if let Some(a) = f.get(1).and_then(|x| x.parse::<ObjId>().ok()) {
                    f[1] = self.mod_net_ref(i, a).unwrap_or_default();
                }
                out.push(format!("w{FIELD_SEP}{}", f.join(&FIELD_SEP.to_string())));
            }
            // In messages of a few hundred characters.
            let mut text = String::from(FX_MESSAGE);
            for s in out {
                let mut f = s.split(FIELD_SEP).skip(1);
                let kind = match (f.next().unwrap_or(""), f.next()) {
                    (k @ ("s" | "c"), Some(what)) => format!("{k} {what}"),
                    (k, _) => k.to_string(),
                };
                let e = self.mods.net_fx.sent_kinds.entry(kind).or_default();
                e.0 += 1;
                e.1 += s.len() + 1;
                if wire_len(&text) + wire_len(&s) + 1 > MESSAGE_MAX && text.len() > FX_MESSAGE.len() {
                    let t = std::mem::replace(&mut text, String::from(FX_MESSAGE));
                    self.mods.net_fx.sent_messages += 1;
                    self.mods.net_fx.sent_bytes += t.len();
                    let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(t), Value::Name("None".into())]);
                }
                text.push_str(&s);
                text.push(STEP_SEP);
            }
            if text.len() > FX_MESSAGE.len() {
                self.mods.net_fx.sent_messages += 1;
                self.mods.net_fx.sent_bytes += text.len();
                let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(text), Value::Name("None".into())]);
            }
        }
        // An actor whose place could not go yet is looked at again next tick.
        self.mods.net_fx.moved = still_moving;
        // What it costs, once a minute in the mods' log.
        let from = *self.mods.net_fx.counted_from.get_or_insert(now);
        if now - from >= 60.0 {
            if self.mods.net_fx.sent_messages > 0 {
                let kinds: Vec<String> = self.mods.net_fx.sent_kinds.iter().map(|(k, (n, b))| format!("{k} {n}/{b}")).collect();
                let line = format!(
                    "{}: scripts' effects sent to the guests over {:.0} s: {} messages, {} bytes ({:.0} bytes a second; steps by kind, count/bytes: {})",
                    self.mod_map_name(),
                    now - from,
                    self.mods.net_fx.sent_messages,
                    self.mods.net_fx.sent_bytes,
                    self.mods.net_fx.sent_bytes as f64 / (now - from),
                    kinds.join(", ")
                );
                self.mod_report(line);
            }
            self.mods.net_fx.sent_messages = 0;
            self.mods.net_fx.sent_bytes = 0;
            self.mods.net_fx.sent_kinds.clear();
            self.mods.net_fx.counted_from = Some(now);
        }
    }

    /// Guest: a ClientMessage carrying effects steps is applied here instead of being shown.
    pub(crate) fn mod_net_fx_message(&mut self, f: crate::types::FuncId, args: &[Option<Value>]) -> bool {
        if !self.functions[f].name.eq_ignore_ascii_case("ClientMessage") {
            return false;
        }
        let Some(Some(Value::Str(text))) = args.first() else { return false };
        let Some(steps) = parse_fx_message(text) else { return false };
        for s in steps {
            self.mod_net_fx_apply(s);
        }
        true
    }

    /// The actor a guest knows by `r` ("c<channel>" or "n<name>").
    fn mod_net_actor_of(&self, r: &str) -> Option<ObjId> {
        if let Some(ch) = r.strip_prefix('c').and_then(|c| c.parse::<usize>().ok()) {
            return self.net_conn_ref(None).and_then(|c| c.actor_channels.get(&ch)).and_then(|a| a.actor);
        }
        let name = r.strip_prefix('n')?;
        self.actors.iter().copied().find(|&a| self.objects.get(a).and_then(|o| o.as_ref()).is_some_and(|o| o.name.eq_ignore_ascii_case(name)))
    }

    /// Guest: one step.
    fn mod_net_fx_apply(&mut self, s: Vec<String>) {
        match s.first().map(String::as_str) {
            Some(e) if e.starts_with('e') => {
                let Ok(ch) = e[1..].parse::<usize>() else { return };
                let actor = self.net_conn_ref(None).and_then(|c| c.actor_channels.get(&ch)).and_then(|a| a.actor);
                match actor {
                    Some(a) => self.mod_net_fx_apply_to(a, &s[1..]),
                    None => self.mods.net_fx.pending.entry(ch).or_default().push(s[1..].to_vec()),
                }
            }
            Some("w") => self.mod_net_level_apply(&s[1..]),
            _ => {}
        }
    }

    /// Guest: a step on effect actor `a`.
    fn mod_net_fx_apply_to(&mut self, a: ObjId, s: &[String]) {
        match s.first().map(String::as_str) {
            Some("s") if s.len() >= 3 => {
                if let (Ok((o, last)), Some(v)) = (crate::mod_script_world::property_path(self, a, &s[1]), decode(&s[2])) {
                    if self.prop_slot(o, &last).is_ok() {
                        self.set_prop(o, &last, v);
                    }
                }
            }
            Some("c") if s.len() >= 2 => {
                if let Some(f) = self.find_function(a, &s[1], false) {
                    let args = vec![Value::Object(None); self.functions[f].params.len()];
                    // On the guest the actor is a proxy, whose functions not marked `simulated`
                    // do not run (an emitter's Trigger): the step runs as the host ran it.
                    let role = self.propr(a, "Role").clone();
                    self.set_prop(a, "Role", Value::Byte(4));
                    let _ = self.call_named(a, &s[1], args);
                    if self.objects.get(a).is_some_and(|o| o.is_some()) {
                        self.set_prop(a, "Role", role);
                    }
                }
            }
            Some("m") if s.len() >= 5 => {
                let at: Vec<f32> = s[3].split(',').filter_map(|x| x.parse().ok()).collect();
                let rot: Vec<i32> = s[4].split(',').filter_map(|x| x.parse().ok()).collect();
                if at.len() != 3 || rot.len() != 3 {
                    return;
                }
                let how = s[1].bytes().next().unwrap_or(b'a');
                let to = if s[2].is_empty() { None } else { self.mod_net_actor_of(&s[2]) };
                if how != b'a' && to.is_none() {
                    return;
                }
                let place = Place { how, to, at: [at[0], at[1], at[2]], rot: [rot[0], rot[1], rot[2]] };
                self.mod_net_fx_place(a, place);
                if how == b'a' {
                    self.mods.net_fx.follow.remove(&a);
                } else {
                    self.mods.net_fx.follow.insert(a, place);
                }
            }
            Some("p") if s.len() >= 2 => {
                let why = format!("{}: from the host", self.mod_map_name());
                let _ = self.mod_play_sound(a, &s[1], &why);
            }
            _ => {}
        }
    }

    /// Guest: a level step ("ch" charred, "ar" charred area, "rt" texture, "h" heat).
    fn mod_net_level_apply(&mut self, s: &[String]) {
        match s.first().map(String::as_str) {
            Some("ch") if s.len() >= 3 => {
                let Ok(k) = s[2].parse::<f32>() else { return };
                match self.mod_net_actor_of(&s[1]) {
                    Some(a) => {
                        if k > 0.0 {
                            self.mod_charred.insert(a, k);
                        } else {
                            self.mod_charred.remove(&a);
                        }
                        let name = self.mod_actor_name(a);
                        self.mods.surfaces.set_charred(&name, k);
                    }
                    None => self.mods.net_fx.pending_level.push(s.to_vec()),
                }
            }
            Some("ar") if s.len() >= 6 => {
                let n: Vec<f32> = s[1..6].iter().filter_map(|x| x.parse().ok()).collect();
                if n.len() == 5 {
                    self.mods.surfaces.char_area([n[0], n[1], n[2]], n[3], n[4]);
                }
            }
            Some("rt") if s.len() >= 4 => self.mods.surfaces.set(&s[1], &s[2], &s[3]),
            Some("h") if s.len() >= 7 => {
                let at: Vec<f32> = s[2].split(',').filter_map(|x| x.parse().ok()).collect();
                let (Ok(radius), Ok(seconds), Ok(strength)) = (s[3].parse::<f32>(), s[4].parse::<f32>(), s[5].parse::<f32>()) else { return };
                let on = if s[1].is_empty() { None } else { self.mod_net_actor_of(&s[1]) };
                let until = self.mod_level_time() + seconds;
                if at.len() == 3 {
                    self.mod_heat_add(crate::mod_heat::HeatZone { name: s[6].clone(), at: [at[0], at[1], at[2]], on, radius, strength, until });
                }
            }
            _ => {}
        }
    }

    /// Guest: effect actor `a` put at `place` (relative to a character as it is now).
    fn mod_net_fx_place(&mut self, a: ObjId, place: Place) {
        let (at, rot) = match (place.how, place.to) {
            (b'v', Some(o)) => {
                let (eyes, vr) = self.mod_net_view(o);
                let ax = view_axes(vr);
                let at = [0, 1, 2].map(|k| eyes[k] + ax[0][k] * place.at[0] + ax[1][k] * place.at[1] + ax[2][k] * place.at[2]);
                (at, [0, 1, 2].map(|k| vr[k] + place.rot[k]))
            }
            (b'o', Some(o)) => {
                let l = self.prop(o, "Location").as_vector();
                ([l[0] + place.at[0], l[1] + place.at[1], l[2] + place.at[2]], place.rot)
            }
            // On the actor as this machine draws it, turned with the view of its character.
            (b'b', Some(t)) => {
                let at = self.mod_pin_point(t, place.at);
                let rot = match self.mod_pin_character(t) {
                    Some(c) => {
                        let vr = self.mod_net_view(c).1;
                        [0, 1, 2].map(|k| vr[k] + place.rot[k])
                    }
                    None => place.rot,
                };
                (at, rot)
            }
            _ => (place.at, place.rot),
        };
        let mut args = [Value::vector(at[0], at[1], at[2])];
        let _ = crate::natives::call(self, "SetLocation", a, &mut args, &[true]);
        let mut args = [Value::rotator(rot[0], rot[1], rot[2])];
        let _ = crate::natives::call(self, "SetRotation", a, &mut args, &[true]);
    }

    /// Guest, every tick: the effects kept relative to a character follow it (a gun's flame at
    /// the player's own view, without waiting for the host).
    pub(crate) fn mod_net_fx_follow(&mut self) {
        if self.mods.net_fx.follow.is_empty() {
            return;
        }
        let list: Vec<(ObjId, Place)> = self.mods.net_fx.follow.iter().map(|(&a, &p)| (a, p)).collect();
        for (a, place) in list {
            let alive = |x: ObjId| self.objects.get(x).is_some_and(|o| o.is_some());
            if !alive(a) || place.to.is_some_and(|o| !alive(o)) {
                self.mods.net_fx.follow.remove(&a);
                continue;
            }
            self.mod_net_fx_place(a, place);
        }
    }

    /// Guest: channel `ch` now carries `actor`: the steps that came before it are applied.
    pub(crate) fn mod_net_fx_channel(&mut self, ch: usize, actor: ObjId) {
        // A new actor with the number of one gone: no place kept from the old one.
        self.mods.net_fx.follow.remove(&actor);
        if let Some(list) = self.mods.net_fx.pending.remove(&ch) {
            for s in list {
                self.mod_net_fx_apply_to(actor, &s);
            }
        }
        if !self.mods.net_fx.pending_level.is_empty() {
            let waiting = std::mem::take(&mut self.mods.net_fx.pending_level);
            for s in waiting {
                self.mod_net_level_apply(&s);
            }
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn network_surface_effects_belong_to_their_game() {
        let mut left = crate::Vm::new();
        let mut right = crate::Vm::new();
        let paint = |vm: &mut crate::Vm, material: &str| {
            vm.mod_net_level_apply(&["rt", "wall", "plaster", material].map(str::to_owned));
        };
        paint(&mut left, "burned");
        paint(&mut right, "wet");
        left.mod_net_level_apply(&["ar", "1", "2", "3", "50", "0.5"].map(str::to_owned));
        let left_view = left.mod_surface_effects().clone();
        let right_view = right.mod_surface_effects().clone();
        assert_eq!(left_view.lookup("wall", "plaster").as_deref(), Some("burned"));
        assert_eq!(right_view.lookup("wall", "plaster").as_deref(), Some("wet"));
        assert_eq!(left_view.char_areas(), [([1.0, 2.0, 3.0], 50.0, 0.5)]);
        assert!(right_view.char_areas().is_empty());
        right.mod_scripts_start();
        assert_eq!(right_view.lookup("wall", "plaster"), None);
        assert_eq!(left_view.lookup("wall", "plaster").as_deref(), Some("burned"));
    }

    use super::*;

    /// An effect made again with the number of one just taken out of the game starts with no
    /// record: every guest gets all of its steps (the jet a host makes again when a guest joins
    /// was never switched on for him). Negative control: another actor's record is kept.
    #[test]
    fn a_number_used_again_starts_clean() {
        let mut vm = Vm::default();
        let place = Place { how: b'a', to: None, at: [0.0; 3], rot: [0; 3] };
        for a in [7usize, 8] {
            vm.mods.net_fx.steps.insert(a, vec!["s`bOwnerNoSee`b1".into(), "s`Emitters[0].Disabled`b1".into()]);
            vm.mods.net_fx.sent.insert((0, a), 2);
            vm.mods.net_fx.place_sent.insert((0, a), (place, 1.0));
            vm.mods.net_fx.charred_sent.insert((0, a), 0.5);
            vm.mods.net_fx.moved.insert(a);
        }
        vm.mod_net_fx_forget(7);
        let fx = &vm.mods.net_fx;
        assert!(!fx.steps.contains_key(&7) && !fx.sent.contains_key(&(0, 7)) && !fx.place_sent.contains_key(&(0, 7)));
        assert!(!fx.charred_sent.contains_key(&(0, 7)) && !fx.moved.contains(&7));
        assert_eq!(fx.sent.get(&(0, 8)), Some(&2));
        assert!(fx.steps.contains_key(&8) && fx.moved.contains(&8));
    }

    /// Without a script effect (no mod, or a mod without scripts), the network hooks do nothing:
    /// the host sends nothing, the guest takes no message and moves nothing, a channel opening
    /// changes nothing. Negative control: one effect noted and the host's sending does run.
    #[test]
    fn inert_without_script_effects() {
        let mut vm = Vm::default();
        vm.mods.script_sources = None;
        assert!(!vm.mod_net_host(), "no network game");
        vm.mod_net_fx_spawned(5);
        assert!(vm.mods.net_fx.steps.is_empty(), "not a host: nothing noted");
        vm.mod_net_send_fx();
        assert!(vm.mods.net_fx.counted_from.is_none() && vm.mods.net_fx.sent_messages == 0, "nothing to send: returns at once");
        vm.mod_net_fx_follow();
        vm.mod_net_fx_channel(3, 7);
        assert!(vm.mods.net_fx.follow.is_empty() && vm.mods.net_fx.pending.is_empty() && vm.mods.net_fx.pending_level.is_empty());
        assert_eq!(parse_fx_message("Hello"), None, "a game's ClientMessage is left to the game");
        vm.mods.net_fx.steps.insert(usize::MAX, Vec::new());
        vm.mod_net_send_fx();
        assert!(vm.mods.net_fx.counted_from.is_none(), "no mod scripts: never sent");
        // Negative control: with a mod's scripts and an effect noted, the host's sending goes on
        // (no guest here: nothing sent).
        vm.mods.script_sources = Some(std::sync::Arc::new(vec![crate::mod_script::ScriptMod { name: "test".into(), folder: std::path::PathBuf::new() }]));
        vm.mod_net_send_fx();
        assert!(vm.mods.net_fx.counted_from.is_some());
    }

    #[test]
    fn values_and_messages_round_trip() {
        for v in [Value::Bool(true), Value::Bool(false), Value::Byte(3), Value::Int(-7), Value::Float(0.5), Value::Str("a b".into())] {
            assert_eq!(decode(&encode(&v).unwrap()).map(|x| format!("{x:?}")), Some(format!("{v:?}")));
        }
        let text = format!("{FX_MESSAGE}e5`s`Emitters[0].Disabled`b1~e5`c`Trigger~w`ch`nStaticMeshActor19`0.750~");
        let steps = parse_fx_message(&text).unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0], vec!["e5", "s", "Emitters[0].Disabled", "b1"]);
        assert_eq!(steps[2], vec!["w", "ch", "nStaticMeshActor19", "0.750"]);
        // A place in a view's axes and back: the same point, for any view.
        for (rot, d) in [([0, 0, 0], [42.0f32, 9.0, -9.0]), ([-3000, 12000, 0], [100.0, -20.0, 5.0]), ([8000, -30000, 0], [1.0, 2.0, 3.0])] {
            let ax = view_axes(rot);
            let world = [0, 1, 2].map(|k| ax[0][k] * d[0] + ax[1][k] * d[1] + ax[2][k] * d[2]);
            let back = [0, 1, 2].map(|k| world[0] * ax[k][0] + world[1] * ax[k][1] + world[2] * ax[k][2]);
            assert!((0..3).all(|k| (back[k] - d[k]).abs() < 1e-3), "{rot:?}: {back:?} for {d:?}");
        }
        // Places close enough are not sent again; further ones are.
        let p = Place { how: b'o', to: Some(3), at: [10.0, 0.0, 0.0], rot: [0, 0, 0] };
        assert!(p.near(&Place { at: [15.0, 0.0, 0.0], ..p }));
        assert!(!p.near(&Place { at: [30.0, 0.0, 0.0], ..p }));
        assert!(!p.near(&Place { rot: [0, 2000, 0], ..p }));
        assert!(!p.near(&Place { how: b'v', ..p }));
        // Pinned to a gun ('b'): sent as often and as finely as a place at a view.
        let b = Place { how: b'b', to: Some(4), at: [104.0, 0.0, 2.0], rot: [0, 0, 0] };
        assert!(b.near(&Place { at: [105.0, 0.0, 2.0], ..b }) && !b.near(&Place { at: [108.0, 0.0, 2.0], ..b }));
        assert!(!b.near(&Place { rot: [400, 0, 0], ..b }) && !b.near(&Place { to: Some(5), ..b }));
        assert_eq!((b.pace(), p.pace()), (0.1, 0.2));
        // Negative controls: other messages are not taken.
        assert_eq!(parse_fx_message("OXIIIMod.Skins.7.XIDCine.Cine2|a"), None);
        assert_eq!(parse_fx_message("Welcome"), None);
        // What the network writes: one byte a character, two for all beyond Latin-1.
        assert_eq!(wire_len("abc"), 3);
        assert_eq!(wire_len("é"), 1);
        assert_eq!(wire_len("aα"), 4);
    }
}
