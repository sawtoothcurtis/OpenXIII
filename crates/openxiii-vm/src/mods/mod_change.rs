//! Mods build only (feature `mods`): `[[map.<Map>.change]]` (one actor placed by the map gets
//! other values and keeps its place and its role), and the `modwho` console command, which
//! names what the player looks at.
//!
//! The changes are written at the start of `begin_play`, before any script of the level runs,
//! like the class values marked `apply_to_placed` (mod_defaults.rs) and the removals
//! (mod_placement.rs): the actor starts its scripts with them. They are made again at each load
//! of the level, and not on a saved level whose actors already carry them.
//!
//! In a network game the server makes the change; a client's own copies of the map's
//! non-static actors are replaced by the server's (net_client_purge), and the model (`Mesh`) is
//! sent with the actor's replicated values. `Skins` is not replicated by the game: a client
//! draws the class's default skins, which is what the server draws when the change gives a look
//! without a `skin`.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use openxiii_mods::looks;
use openxiii_mods::placement::{Change, Edit};
use openxiii_mods::ModValue;

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// Largest angle (degrees) between the view and an actor `modwho` names without hitting it.
const MODWHO_CONE_DEGREES: f32 = 6.0;
/// How far `modwho` looks.
const MODWHO_DISTANCE: f32 = 20000.0;

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Unit vector of a rotation (pitch, yaw in game units).
fn direction(pitch: i32, yaw: i32) -> [f32; 3] {
    let to_rad = |u: i32| (u as f32) * std::f32::consts::TAU / 65536.0;
    let (p, y) = (to_rad(pitch), to_rad(yaw));
    [p.dcos() * y.dcos(), p.dcos() * y.dsin(), p.dsin()]
}

impl Vm {
    fn mod_alive_actor(&self, a: ObjId) -> bool {
        self.objects.get(a).is_some_and(|o| o.is_some()) && !self.propr(a, "bDeleteMe").as_bool()
    }

    /// The actors `who` names (case ignored): by name, else the characters whose Tag or
    /// PawnName it is, else the other actors whose Tag it is (the first of these that finds
    /// any); then only those within `radius` of `near`, if given.
    pub(crate) fn mod_find_placed(&self, who: &str, near: Option<[f32; 3]>, radius: f32) -> Vec<ObjId> {
        let (found, stage) = self.mod_find_placed_stages(who, near, radius);
        let _ = stage;
        found
    }

    /// [`Vm::mod_find_placed`], and which test found the actors: 0 by name, 1 characters by Tag
    /// or PawnName, 2 other actors by Tag, 3 none.
    fn mod_find_placed_stages(&self, who: &str, near: Option<[f32; 3]>, radius: f32) -> (Vec<ObjId>, u8) {
        let live: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.mod_alive_actor(a)).collect();
        let by_name: Vec<ObjId> = live.iter().copied().filter(|&a| self.objects[a].as_ref().is_some_and(|o| o.name.eq_ignore_ascii_case(who))).collect();
        let mut stage = 0;
        let found = if !by_name.is_empty() {
            by_name
        } else {
            // Characters first: a scene's other actors (its dispatcher, triggers) often share
            // the character's Tag.
            let tag = |a: ObjId| self.prop(a, "Tag").as_name().eq_ignore_ascii_case(who);
            let characters: Vec<ObjId> =
                live.iter().copied().filter(|&a| self.is_a(a, "Pawn") && (tag(a) || self.prop(a, "PawnName").as_name().eq_ignore_ascii_case(who))).collect();
            if !characters.is_empty() {
                stage = 1;
                characters
            } else {
                let others: Vec<ObjId> = live.iter().copied().filter(|&a| tag(a)).collect();
                stage = if others.is_empty() { 3 } else { 2 };
                others
            }
        };
        let found = match near {
            Some(p) => found.into_iter().filter(|&a| distance(self.native_value(a, Field::Location).as_vector(), p) <= radius).collect(),
            None => found,
        };
        (found, stage)
    }

    /// Names a modder can write in `who` (actors' names, Tags and characters' PawnNames), for
    /// "did you mean".
    fn mod_who_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for &a in &self.actors {
            if !self.mod_alive_actor(a) {
                continue;
            }
            let pawn = self.is_a(a, "Pawn");
            // Characters and the actors scripts name; not every piece of decor.
            if !pawn && self.propr(a, "bStatic").as_bool() {
                continue;
            }
            if let Some(o) = self.objects[a].as_ref() {
                out.push(o.name.clone());
            }
            let tag = self.prop(a, "Tag").as_name();
            if !tag.is_empty() && !tag.eq_ignore_ascii_case("None") {
                out.push(tag);
            }
            if pawn {
                let n = self.prop(a, "PawnName").as_name();
                if !n.is_empty() {
                    out.push(n);
                }
            }
        }
        out.sort_by_key(|s| s.to_lowercase());
        out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        out
    }

    /// The game's model `path` names: Ok(its class) or Err(why not).
    fn mod_model_class(&mut self, path: &str) -> Result<String, String> {
        if openxiii_assets::mod_skeletal::get(path).is_some() {
            return Ok("SkeletalMesh".into());
        }
        let found = self.library.as_mut().and_then(|lib| lib.resolve(path).map(|(lp, i)| lp.package.export_class(i).to_string()));
        match found {
            Some(c) if c.to_lowercase().contains("mesh") => Ok(c),
            Some(c) => Err(format!("{path} is a {c}, not a character model")),
            None => Err(format!("{path} is not an object of the game's packages")),
        }
    }

    /// Start of begin_play, after the class values: the `change` entries of the map.
    pub(crate) fn apply_mod_changes(&mut self) {
        if self.mod_net_client() || self.level.is_some_and(|l| self.native_prop(l, Field::BegunPlay).as_bool()) {
            return;
        }
        let map = self.mod_map_name();
        for e in self.mod_edits_here() {
            let Edit::Change(c) = &e.edit else { continue };
            let what = format!("{map}: {}", c.describe());
            let found = self.mod_find_placed(&c.who, c.near, c.radius);
            if found.is_empty() {
                let near = if c.near.is_some() && !self.mod_find_placed(&c.who, None, 0.0).is_empty() { " near that position" } else { "" };
                let close = looks::closest(&c.who, self.mod_who_names().iter().map(String::as_str), 3);
                self.mod_report(format!("warning: {what}: no actor named {}{near} in the map{} ({})", c.who, looks::did_you_mean(&close), e.source));
                continue;
            }
            // The model, checked once for all the actors.
            let model = match self.mod_change_model(c) {
                Ok(m) => m,
                Err(why) => {
                    self.mod_report(format!("warning: {what}: {why}; look not changed ({})", e.source));
                    None
                }
            };
            // team and can_die: the derived [character] the actor becomes (simple.rs).
            let character =
                c.character.as_ref().and_then(|n| self.mods.behaviour.as_ref().and_then(|b| b.characters.iter().position(|d| d.name.eq_ignore_ascii_case(n))));
            let mut names = Vec::new();
            for &a in &found {
                if let Some(i) = character {
                    if self.is_a(a, "Pawn") {
                        self.mods.changed_characters.push((a, i));
                    } else {
                        self.mod_report(format!("warning: {what}: {} is not a character: team and can_die do nothing ({})", self.mod_actor_label(a), e.source));
                    }
                }
                let errors = self.mod_change_actor(a, c, model.as_deref());
                for why in errors {
                    self.mod_report(format!("warning: {what}: {}: {why} ({})", self.mod_actor_label(a), e.source));
                }
                names.push(self.mod_actor_label(a));
            }
            let many = if found.len() > 1 { format!(" ({} actors match; add near = [x, y, z] to pick one)", found.len()) } else { String::new() };
            self.mod_report(format!("{what}: {}{many} ({})", names.join(", "), e.source));
        }
    }

    /// End of the level start-up: the placed characters a `change` gives a team or can_die
    /// become mod characters (their AI controllers now exist).
    pub(crate) fn take_mod_changed_characters(&mut self) {
        for (a, i) in std::mem::take(&mut self.mods.changed_characters) {
            let Some(def) = self.mods.behaviour.as_ref().and_then(|b| b.characters.get(i).cloned()) else { continue };
            if self.mod_alive_actor(a) {
                self.mod_take_character(a, i, &def);
            }
        }
    }

    /// "Cine2 (XIDCine.Cine2)".
    fn mod_actor_label(&self, a: ObjId) -> String {
        match self.objects.get(a).and_then(|o| o.as_ref()) {
            Some(o) => format!("{} ({})", o.name, self.class_path(o.class)),
            None => format!("#{a}"),
        }
    }

    /// The model of change `c` (its `look`), checked: Ok(None) without a look, Err(why) for a
    /// look that is not a character model (with the closest names). Also used by the scripts'
    /// `change` (mod_script_game.rs).
    pub(crate) fn mod_change_model(&mut self, c: &Change) -> Result<Option<String>, String> {
        let Some(l) = &c.look else { return Ok(None) };
        match looks::resolve(l).map_err(|close| format!("{l} is not a character model{}", looks::did_you_mean(&close))).and_then(|m| self.mod_model_class(&m).map(|_| m)) {
            Ok(m) => Ok(Some(m)),
            Err(why) => {
                let close = looks::closest(l, looks::look_names(), 3);
                let hint = if why.contains("did you mean") { String::new() } else { looks::did_you_mean(&close) };
                Err(format!("{why}{hint}"))
            }
        }
    }

    /// Writes change `c` on actor `a`. Returns why a value could not be written.
    pub(crate) fn mod_change_actor(&mut self, a: ObjId, c: &Change, model: Option<&str>) -> Vec<String> {
        let Some(class) = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.class) else { return Vec::new() };
        let mut values: Vec<(String, Option<usize>, ModValue)> = Vec::new();
        if let Some(m) = model {
            values.push(("Mesh".into(), None, ModValue::Str(m.to_string())));
        }
        if let Some(h) = c.health {
            values.push(("Health".into(), None, ModValue::Int(h as i64)));
        }
        for p in &c.properties {
            values.push((p.property.clone(), p.index, p.value.clone()));
        }
        // The skins: the one asked for, or, with a new look, the class's default ones (the map's
        // own skins were made for the old model).
        let skins = match (&c.skin, model) {
            (Some(s), _) => match self.mod_asset_value("Material", s) {
                Some(Ok(v)) => Some(Ok(Value::Array(vec![v]))),
                Some(Err(why)) => Some(Err(format!("skin: {why}"))),
                None => Some(Err(format!("skin: {s} is not a material"))),
            },
            (None, Some(_)) => {
                self.create_cdo(class);
                Some(Ok(self.prop(self.classes[class].cdo, "Skins")))
            }
            (None, None) => None,
        };
        let mut errors = Vec::new();
        let Some(mut props) = self.objects[a].as_mut().map(|o| std::mem::take(&mut o.props)) else { return errors };
        for (property, index, value) in &values {
            if let Err(why) = props.with_slice(|s| self.mod_set_value(class, s, property, *index, value)) {
                let label = match index {
                    Some(i) => format!("{property}[{i}]"),
                    None => property.clone(),
                };
                errors.push(format!("{label}: {why}, value ignored"));
            }
        }
        if let Some(o) = self.objects[a].as_mut() {
            o.props = props;
        }
        match skins {
            Some(Ok(v)) if self.prop_slot(a, "Skins").is_ok() => self.set_prop(a, "Skins", v),
            Some(Ok(_)) => {}
            Some(Err(why)) => errors.push(format!("{why}, value ignored")),
            None => {}
        }
        // Skins chosen by the mod (not the class's own after a new look): sent to the network
        // guests, which do not receive skins from the game (mod_net_skins.rs).
        if c.skin.is_some() || c.properties.iter().any(|p| p.property.eq_ignore_ascii_case("Skins")) {
            self.mod_net_note_skins(a);
        }
        errors
    }

    /// `modwho`: what the player looks at (the actor hit, or else the one closest to the
    /// middle of the view), with its name, class, PawnName, Tag, Mesh and Health, and a
    /// `[[map.<Map>.change]]` entry ready to paste. Also written to the log and mods-log.txt.
    pub(crate) fn mod_who(&mut self, pc: ObjId) -> String {
        let Some(pawn) = self.native_prop(pc, Field::Pawn).as_object() else {
            return "modwho: no player".into();
        };
        let at = self.native_value(pawn, Field::Location).as_vector();
        let eye = self.eye_position(pawn);
        let start = [at[0] + eye[0], at[1] + eye[1], at[2] + eye[2]];
        let r = self.native_value(pc, Field::Rotation).as_rotator();
        let dir = direction(r[0], r[1]);
        let end = [start[0] + dir[0] * MODWHO_DISTANCE, start[1] + dir[1] * MODWHO_DISTANCE, start[2] + dir[2] * MODWHO_DISTANCE];
        let hit = self.line_check(pawn, start, end, [0.0; 3], true);
        let wall = hit.map_or(MODWHO_DISTANCE, |(t, _, _)| t * MODWHO_DISTANCE);
        let level_info = self.level;
        let hit_actor = hit.and_then(|(_, _, a)| a).filter(|&a| Some(a) != level_info && !self.propr(a, "bStatic").as_bool() && !self.is_a(a, "LevelInfo"));
        // Characters often let shots and traces through (cutscene characters): the visible actor
        // closest to the middle of the view, in front of the wall.
        let target = hit_actor.or_else(|| {
            let cos = MODWHO_CONE_DEGREES.to_radians().dcos();
            let mut best: Option<(f32, ObjId)> = None;
            for &a in &self.actors {
                if a == pawn
                    || a == pc
                    || !self.mod_alive_actor(a)
                    || self.native_prop(a, Field::Hidden).as_bool()
                    || self.native_prop(a, Field::DrawType).as_byte() == 0
                    || self.propr(a, "bStatic").as_bool()
                {
                    continue;
                }
                let p = self.native_value(a, Field::Location).as_vector();
                let v = [p[0] - start[0], p[1] - start[1], p[2] - start[2]];
                let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if d < 1.0 || d > wall + 100.0 {
                    continue;
                }
                let c = (v[0] * dir[0] + v[1] * dir[1] + v[2] * dir[2]) / d;
                // Characters first: an angle a little wider is fine for them.
                let c = if self.is_a(a, "Pawn") { c + 0.002 } else { c };
                if c >= cos && best.is_none_or(|(bc, _)| c > bc) {
                    best = Some((c, a));
                }
            }
            best.map(|(_, a)| a)
        });
        let map = self.mod_map_name();
        let text = match target {
            None => format!("# {map}: nothing in front of the player"),
            Some(a) => {
                let name = self.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default();
                let class = self.objects[a].as_ref().map(|o| self.class_path(o.class)).unwrap_or_default();
                let pawn_name = if self.is_a(a, "Pawn") { self.prop(a, "PawnName").as_name() } else { String::new() };
                let tag = self.prop(a, "Tag").as_name();
                let mesh = match self.prop(a, "Mesh") {
                    Value::Asset(p) => p,
                    Value::Object(Some(o)) => self.objects.get(o).and_then(|x| x.as_ref()).map(|x| x.name.clone()).unwrap_or_default(),
                    _ => "None".into(),
                };
                let health = match self.prop(a, "Health") {
                    Value::Int(h) => h.to_string(),
                    _ => "-".into(),
                };
                let p = self.native_value(a, Field::Location).as_vector();
                // The same `who` as the names catalog: the PawnName or Tag when it finds this actor
                // alone, else its object name (unique in the map).
                let who = self.friendly_who(a);
                let mut text = format!(
                    "# {map}: {name} ({class}), PawnName {pawn_name:?}, Tag {tag:?}, Mesh {mesh}, Health {health}, at [{:.0}, {:.0}, {:.0}]\n",
                    p[0], p[1], p[2]
                );
                text.push_str(&format!("[[map.{map}.change]]\nwho = \"{who}\"\nlook = \"Jones\"   # the new look: a name of docs/modding/names/characters.md or of the names catalog"));
                text
            }
        };
        for line in text.lines() {
            let l = format!("modwho: {line}");
            openxiii_mods::log::note(&l);
            self.log.push(l);
        }
        text
    }
}

