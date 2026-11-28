//! Mods build only (feature `mods`): values changed by mods (`[class."Package.Class"]` and
//! `[map.<Map>.class."Package.Class"]` tables, docs/modding/README.md).
//!
//! Class defaults: applied at the very end of `cdo_values`, after the serialized
//! defaultproperties, LoadConfig and LoadLocalized, so they also survive the reload of class
//! defaults at a level change (`reload_class_defaults` goes through `cdo_values`).
//!
//! Placed actors: an actor placed in a map starts from its class defaults, then gets the values
//! the level designer saved with it in the map, which class defaults cannot reach. At the start
//! of `begin_play`, before any script of the level runs, the values marked `apply_to_placed` and
//! the values of the current map are also written on the map's actors of the class and its
//! subclasses.
//!
//! Each value is converted to the declared property type with the engine's own ImportText, as
//! for .ini values. A value that cannot be used gives one warning line in the log and is skipped.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use openxiii_mods::{DefaultOverride, ModValue};

use crate::types::Type;
use crate::value::{ClassId, Value};
use crate::Vm;

/// Values set by mods, by lower-case "package.class", in mod order.
#[derive(Debug, Default)]
pub struct ModTable {
    /// Class defaults (some also marked `apply_to_placed`).
    classes: BTreeMap<String, Vec<DefaultOverride>>,
    /// Per lower-case map name: values for the actors of that map.
    maps: BTreeMap<String, BTreeMap<String, Vec<DefaultOverride>>>,
    /// Some value is a mod-only weapon setting (mod_weapons.rs).
    mod_only: bool,
}

impl ModTable {
    fn is_empty(&self) -> bool {
        self.classes.is_empty() && self.maps.is_empty()
    }

    /// Some value is a mod-only weapon setting (FireAnimRate…).
    pub(crate) fn has_mod_only(&self) -> bool {
        self.mod_only
    }

    /// Class default values of lower-case "package.class" `key`.
    pub(crate) fn class_values(&self, key: &str) -> Option<&Vec<DefaultOverride>> {
        self.classes.get(key)
    }

    /// The tables for map `map` (by lower-case "package.class"), the most precise selector first
    /// (openxiii_mods::maps).
    pub(crate) fn map_class_all(&self, map: &str) -> Vec<&BTreeMap<String, Vec<DefaultOverride>>> {
        let mut v = self.map(map);
        v.reverse();
        v
    }

    /// The tables of the selectors that are for map `map` (a map name, "Hual*", "solo", "*"…),
    /// the least precise first: written in this order, the most precise value wins.
    fn map(&self, map: &str) -> Vec<&BTreeMap<String, Vec<DefaultOverride>>> {
        let mut v: Vec<(u32, &String, &BTreeMap<String, Vec<DefaultOverride>>)> = self.maps.iter().filter_map(|(sel, m)| openxiii_mods::maps::rank(sel, map).map(|r| (r, sel, m))).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        v.into_iter().map(|(_, _, m)| m).collect()
    }

    /// All the class default values.
    pub(crate) fn classes_all(&self) -> impl Iterator<Item = &DefaultOverride> {
        self.classes.values().flatten()
    }

    /// The values for map `map`, the least precise selector first.
    pub(crate) fn maps_of<'a>(&'a self, map: &str) -> impl Iterator<Item = &'a DefaultOverride> {
        self.map(map).into_iter().flat_map(|m| m.values().flatten())
    }
}

pub type ModDefaults = Arc<ModTable>;

static GLOBAL: OnceLock<ModDefaults> = OnceLock::new();
static REPORTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn table(list: Vec<DefaultOverride>) -> ModDefaults {
    let mut t = ModTable::default();
    t.mod_only = list.iter().any(|o| crate::mod_weapons::is_mod_only(&o.property));
    for o in list {
        match &o.map {
            Some(m) => t.maps.entry(m.to_lowercase()).or_default().entry(o.class_key()).or_default().push(o),
            None => t.classes.entry(o.class_key()).or_default().push(o),
        }
    }
    Arc::new(t)
}

/// Sets the values used by every Vm created afterwards (once, at startup, before any game
/// script is loaded). A second call is ignored.
pub fn set_mod_defaults(list: Vec<DefaultOverride>) {
    let _ = GLOBAL.set(table(list));
}

pub(crate) fn global_mod_defaults() -> Option<ModDefaults> {
    GLOBAL.get().cloned().filter(|t| !t.is_empty())
}

impl Vm {
    /// Values of this Vm only (tests). Must be called before the classes are loaded.
    pub fn set_mod_defaults_here(&mut self, list: Vec<DefaultOverride>) {
        self.mods.defaults = Some(table(list)).filter(|t| !t.is_empty());
    }

    /// One log line, printed once per run (several Vms and reloads apply the same values), and
    /// kept in `mod_messages` for this Vm.
    pub(crate) fn mod_report(&mut self, line: String) {
        let first = {
            let mut g = REPORTED.lock().unwrap_or_else(|e| e.into_inner());
            g.get_or_insert_with(HashSet::new).insert(line.clone())
        };
        if first {
            println!("mods: {line}");
            openxiii_mods::log::note(&line);
        }
        self.mod_messages.push(line);
    }

    /// Name of the map being played ("Plage01"), from the URL.
    pub(crate) fn mod_map_name(&self) -> String {
        let m = self.url.map.rsplit(['/', '\\']).next().unwrap_or_default();
        m.split('.').next().unwrap_or_default().to_string()
    }

    /// Classes named by mods (global, and for the current map) that are not loaded: unknown
    /// class (its package is loaded) or package not loaded (yet). Called by the game once the
    /// first map is loaded.
    pub fn report_mod_classes(&mut self) {
        let Some(t) = self.mods.defaults.clone() else { return };
        let map = self.mod_map_name();
        let mut lists: Vec<(&String, &Vec<DefaultOverride>)> = t.classes.iter().collect();
        for m in t.map(&map) {
            lists.extend(m.iter());
        }
        lists.sort_by_key(|(k, l)| (l[0].map.is_some(), (*k).clone()));
        for (key, list) in lists {
            if self.class_qualified.contains_key(key) {
                continue;
            }
            let o = &list[0];
            let package = o.class.split('.').next().unwrap_or_default();
            let line = if self.packages.iter().any(|p| p.name.eq_ignore_ascii_case(package)) {
                format!("warning: unknown class {} (package {package} has no such class), its values are ignored ({})", o.scope_label(), o.source())
            } else {
                format!("warning: class {}: package {package} is not loaded, its values will apply if it is loaded later ({})", o.scope_label(), o.source())
            };
            self.mod_report(line);
        }
    }

    /// Values of `t` that concern an object of class `class`: global ones (all, or only those
    /// marked `apply_to_placed`), then those of `map`; parent classes first, so the most
    /// specific value is written last and wins.
    fn mod_values_for<'t>(&self, t: &'t ModTable, class: ClassId, placed_only: bool, map: Option<&str>) -> Vec<&'t DefaultOverride> {
        let mut chain = Vec::new();
        let mut k = Some(class);
        while let Some(c) = k {
            chain.push(self.class_path(c).to_lowercase());
            k = self.classes[c].super_class;
        }
        chain.reverse();
        let mut out = Vec::new();
        for key in &chain {
            if let Some(list) = t.classes.get(key) {
                out.extend(list.iter().filter(|o| !placed_only || o.apply_to_placed));
            }
        }
        // The least precise selector first ("*", "solo", "Hual*", then the map's name): the most
        // precise value is written last and wins.
        for m in map.map(|m| t.map(m)).unwrap_or_default() {
            for key in &chain {
                if let Some(list) = m.get(key) {
                    out.extend(list.iter());
                }
            }
        }
        out
    }

    /// Debug dump (OXIII_MODS_DUMP=1): every object (class defaults and live objects) whose class
    /// or a parent class has mod values, with the current value of each of those properties.
    pub fn mod_dump(&self) -> Vec<String> {
        let Some(t) = self.mods.defaults.clone() else { return Vec::new() };
        let map = self.mod_map_name();
        let mut out = Vec::new();
        for (id, o) in self.objects.iter().enumerate() {
            let Some(o) = o else { continue };
            let overrides = self.mod_values_for(&t, o.class, false, Some(&map));
            if overrides.is_empty() {
                continue;
            }
            let owner = match self.prop(id, "Owner") {
                Value::Object(Some(w)) => self.objects.get(w).and_then(|x| x.as_ref()).map(|x| format!(", owner {}", x.name)).unwrap_or_default(),
                _ => String::new(),
            };
            // One entry per property (the last value written wins).
            let mut vals: BTreeMap<String, String> = BTreeMap::new();
            for ov in overrides {
                // A mod-only setting: the value that applies to this object.
                if crate::mod_weapons::is_mod_only(&ov.property) {
                    if let Some(o) = self.mod_weapon_setting(self.objects[id].as_ref().map_or(0, |x| x.class), &ov.property) {
                        vals.insert(o.property_label(), format!("{}", o.value));
                    }
                    continue;
                }
                let v = self.prop(id, &ov.property);
                let v = match (ov.index, &v) {
                    (Some(i), Value::Array(items)) => items.get(i).cloned().unwrap_or(Value::Void),
                    _ => v,
                };
                vals.insert(ov.property_label(), format!("{v}"));
            }
            let vals: Vec<String> = vals.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
            out.push(format!("{} ({}{owner}): {}", o.name, self.class_path(o.class), vals.join(", ")));
        }
        out
    }

    /// End of cdo_values: the mods' class default values for class `c`.
    pub(crate) fn apply_mod_defaults(&mut self, c: ClassId, values: &mut [Value]) {
        let Some(t) = self.mods.defaults.clone() else { return };
        let class_path = self.class_path(c);
        // An attachment class whose weapon's FiringMode a mod changed (mod_weapons.rs).
        self.apply_attach_firing_mode(c, values);
        // A mod's new weapon class: a weapon slot of its own (mod_weapons.rs).
        self.mod_weapon_slot(c, values);
        let Some(list) = t.classes.get(&class_path.to_lowercase()) else { return };
        // A class given its own StaticMeshName (an InventoryAttachment: the gun seen in a hand)
        // loads that mesh only when it has no StaticMesh (InventoryAttachment.PostBeginPlay).
        // The StaticMesh it starts from can be one a game script wrote into its parent's
        // defaults (PostBeginPlay's `default.StaticMesh = StaticMesh`, as soon as a gun of the
        // parent class was held: the .44's revolver for a mod's gun made from MagnumAttach),
        // and that mesh would be drawn instead. Unless the mod also gives a StaticMesh, the
        // class starts with none.
        let given = |p: &str| list.iter().any(|o| o.map.is_none() && o.property.eq_ignore_ascii_case(p));
        if given("StaticMeshName") && !given("StaticMesh") {
            if let Some(slot) = self.classes[c].prop_index.get("staticmesh").and_then(|&s| values.get_mut(s)) {
                *slot = Value::Object(None);
            }
        }
        // FirstPersonFrom first: the values the mod gives itself win over the copied look.
        let mut list: Vec<&DefaultOverride> = list.iter().collect();
        list.sort_by_key(|o| !o.property.eq_ignore_ascii_case(crate::mod_weapons::FIRST_PERSON_FROM));
        for o in list {
            if crate::mod_weapons::is_mod_only(&o.property) {
                self.apply_mod_only(c, values, o);
                continue;
            }
            // Sub-object values while the packages' sub-objects are not indexed yet (first
            // class defaults of load_dir): applied by apply_mod_subobjects right after.
            if self.script_exports.is_empty() && self.is_subobject_value(c, o) {
                continue;
            }
            match self.mod_set(c, values, o) {
                Ok(()) => {
                    self.mod_report(format!("{class_path}.{} = {} ({})", o.property_label(), o.value, o.source()));
                    if o.property.eq_ignore_ascii_case("FiringMode") {
                        self.follow_firing_mode(c, values, o);
                    }
                }
                Err(why) => self.mod_report(format!("warning: {class_path}.{}: {why} ({}), value ignored", o.property_label(), o.source())),
            }
        }
    }

    /// True for an inline table given to an object property (values of a sub-object).
    fn is_subobject_value(&self, c: ClassId, o: &DefaultOverride) -> bool {
        let ModValue::Table(_) = o.value else { return false };
        let Some(&slot) = self.classes[c].prop_index.get(&o.property.to_lowercase()) else { return false };
        match &self.classes[c].props[slot].ty {
            Type::Object(_) => true,
            Type::Array(inner) => matches!(**inner, Type::Object(_)),
            _ => false,
        }
    }

    /// load_dir, once the packages' sub-objects are indexed: the sub-object values
    /// (`"Emitters[1]" = { … }`) left aside by the first class defaults, written into them.
    pub(crate) fn apply_mod_subobjects(&mut self) {
        let Some(t) = self.mods.defaults.clone() else { return };
        for (key, list) in t.classes.iter() {
            let Some(&c) = self.class_qualified.get(key) else { continue };
            let list: Vec<&DefaultOverride> = list.iter().filter(|o| self.is_subobject_value(c, o)).collect();
            let cdo = self.classes[c].cdo;
            if list.is_empty() || cdo == usize::MAX {
                continue;
            }
            let class_path = self.class_path(c);
            let Some(mut props) = self.objects.get_mut(cdo).and_then(|x| x.as_mut()).map(|x| std::mem::take(&mut x.props)) else { continue };
            for o in list {
                match props.with_slice(|v| self.mod_set(c, v, o)) {
                    Ok(()) => self.mod_report(format!("{class_path}.{} = {} ({})", o.property_label(), o.value, o.source())),
                    Err(why) => self.mod_report(format!("warning: {class_path}.{}: {why} ({}), value ignored", o.property_label(), o.source())),
                }
            }
            if let Some(x) = self.objects[cdo].as_mut() {
                x.props = props;
            }
        }
    }

    /// Start of begin_play: values marked `apply_to_placed`, then those of the current map,
    /// written on the actors the map placed. One summary line per value.
    pub(crate) fn apply_mod_placed(&mut self) {
        let Some(t) = self.mods.defaults.clone() else { return };
        // A saved game being loaded (bBegunPlay): its actors already carry the values written
        // when the level first started, plus what happened since (a wounded soldier stays
        // wounded).
        if self.level.is_some_and(|l| self.native_prop(l, crate::NativeField::BegunPlay).as_bool()) {
            return;
        }
        let map = self.mod_map_name();
        // (value, actors changed, actors that had their own value in the map), in first-seen order.
        let mut counts: Vec<(&DefaultOverride, usize, usize)> = Vec::new();
        let mut errors: Vec<(&DefaultOverride, String)> = Vec::new();
        for a in self.actors.clone() {
            let Some(class) = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.class) else { continue };
            let mut list = self.mod_values_for(&t, class, true, Some(&map));
            list.retain(|o| !crate::mod_weapons::is_mod_only(&o.property));
            if list.is_empty() {
                continue;
            }
            self.create_cdo(class);
            let cdo = self.classes[class].cdo;
            let Some(mut props) = self.objects[a].as_mut().map(|o| std::mem::take(&mut o.props)) else { continue };
            // Properties the map saved with their own value, read before anything is changed.
            let own_values: HashSet<String> = props.with_slice(|v| {
                list.iter()
                    .map(|o| o.property.to_lowercase())
                    .filter(|name| self.classes[class].prop_index.get(name).and_then(|&s| v.get(s)).is_some_and(|x| *x != self.prop(cdo, name)))
                    .collect()
            });
            for o in list {
                let own = own_values.contains(&o.property.to_lowercase());
                match props.with_slice(|v| self.mod_set(class, v, o)) {
                    Ok(()) => match counts.iter_mut().find(|(x, _, _)| std::ptr::eq(*x, o)) {
                        Some(e) => {
                            e.1 += 1;
                            e.2 += own as usize;
                        }
                        None => counts.push((o, 1, own as usize)),
                    },
                    Err(why) => {
                        if !errors.iter().any(|(x, _)| std::ptr::eq(*x, o)) {
                            errors.push((o, why));
                        }
                    }
                }
            }
            if let Some(obj) = self.objects[a].as_mut() {
                obj.props = props;
            }
        }
        for (o, n, own) in counts {
            let scope = if o.map.is_some() { format!("{map}: ") } else { String::new() };
            self.mod_report(format!(
                "{scope}{}.{} = {} on {n} actor(s) placed in {map}, {own} of them with their own value in the map ({})",
                o.class,
                o.property_label(),
                o.value,
                o.source()
            ));
        }
        for (o, why) in errors {
            self.mod_report(format!("warning: {map}: {}.{}: {why} ({}), value ignored", o.class, o.property_label(), o.source()));
        }
    }

    /// Converts and writes value `o` into `values` (properties of class `c`). Err: why not.
    fn mod_set(&mut self, c: ClassId, values: &mut [Value], o: &DefaultOverride) -> Result<(), String> {
        self.mod_set_value(c, values, &o.property, o.index, &o.value)
    }

    /// Converts and writes `value` into property `property` (element `index` of a static array)
    /// of `values`, the properties of an object of class `c`. Err: why not.
    pub(crate) fn mod_set_value(&mut self, c: ClassId, values: &mut [Value], property: &str, index: Option<usize>, value: &ModValue) -> Result<(), String> {
        let Some(&slot) = self.classes[c].prop_index.get(&property.to_lowercase()) else {
            return Err(format!("{} has no property {property}", self.class_path(c)));
        };
        let p = self.classes[c].props[slot].clone();
        // An inline table for an object reference: values of the sub-object it holds.
        if let ModValue::Table(fields) = value {
            let object_list = matches!(&p.ty, Type::Array(inner) if matches!(**inner, Type::Object(_)));
            if object_list || matches!(p.ty, Type::Object(_)) {
                return self.mod_set_subobject(c, values, slot, object_list, index, fields);
            }
        }
        // A list of asset references (an actor's Skins): each one found as a single asset is,
        // in any package of the game or a mod's own (the text reader only knows the loaded ones).
        if let (Type::Array(inner), ModValue::Array(items), 1, None) = (&p.ty, value, p.dim, index) {
            if let Type::Object(meta) = &**inner {
                let mut list = Vec::new();
                for item in items {
                    let ModValue::Str(s) = item else { break };
                    match self.mod_asset_value(meta, s) {
                        Some(Ok(v)) => list.push(v),
                        Some(Err(why)) => return Err(why),
                        None => break,
                    }
                }
                if list.len() == items.len() {
                    values[slot] = Value::Array(list);
                    return Ok(());
                }
            }
        }
        let index = match (p.dim, index) {
            (1, None) => None,
            (1, Some(_)) => return Err(format!("{} is not a static array, write it without [index]", p.name)),
            (d, None) => return Err(format!("{} is a static array of {d} elements, write \"{}[0]\"…\"{}[{}]\"", p.name, p.name, p.name, d - 1)),
            (d, Some(i)) if i >= d => return Err(format!("index out of range, the array has {d} elements")),
            (_, Some(i)) => Some(i),
        };
        let text = self.mod_text(&p.ty, p.enumeration.as_deref(), value, false)?;
        // Sounds, textures, meshes…: objects of the game's packages, not script objects.
        if let Type::Object(meta) = &p.ty {
            if let Some(r) = self.mod_asset_value(meta, &text) {
                let v = r?;
                match index {
                    None => values[slot] = v,
                    Some(i) => {
                        if let Value::Array(items) = &mut values[slot] {
                            items[i] = v;
                        }
                    }
                }
                return Ok(());
            }
        }
        let current = match index {
            None => values.get(slot).cloned(),
            Some(i) => match values.get(slot) {
                Some(Value::Array(items)) => items.get(i).cloned(),
                _ => None,
            },
        };
        let Some(mut v) = current else { return Err("no value slot".into()) };
        let chars: Vec<char> = text.chars().collect();
        let ok = self.import_text_named(&p.name, &p.ty, p.enumeration.as_deref(), &chars, 0, 0, &mut v).is_some();
        let missing_ref = matches!((&p.ty, &v), (Type::Object(_), Value::Object(None)) | (Type::Class(_), Value::Class(None))) && !text.eq_ignore_ascii_case("None");
        if missing_ref {
            return Err("object or class not found".into());
        }
        if !ok {
            return Err(format!("the game could not read {text:?} as {}", type_name(&p.ty)));
        }
        match index {
            None => values[slot] = v,
            Some(i) => {
                if let Value::Array(items) = &mut values[slot] {
                    items[i] = v;
                }
            }
        }
        Ok(())
    }

    /// An inline table given to an object property of class `c` (element `index` of a list such as
    /// an Emitter's `Emitters`, or of a static array): the object it holds is a sub-object (the
    /// particle emitters of an Emitter class are objects of that class's package), so class `c`
    /// gets its own copy of it with the table's values, and the original stays as it is for the
    /// classes that share it (a mod's new class and its parent). The actors of class `c` then
    /// copy it when they are spawned, as the game copies the parent's (particles.rs,
    /// construct_emitters). Err: why not.
    fn mod_set_subobject(&mut self, c: ClassId, values: &mut [Value], slot: usize, object_list: bool, index: Option<usize>, fields: &[(String, ModValue)]) -> Result<(), String> {
        let p = self.classes[c].props[slot].clone();
        let current = match (object_list || p.dim > 1, index, values.get(slot)) {
            (false, None, Some(v)) => v.clone(),
            (false, Some(_), _) => return Err(format!("{} is not a list, write it without [index]", p.name)),
            (true, None, _) => return Err(format!("{} is a list of objects: write \"{}[0]\" = {{ … }} for the values of its first one", p.name, p.name)),
            (true, Some(i), Some(Value::Array(items))) => items.get(i).cloned().ok_or_else(|| format!("index out of range, {} has {} element(s)", p.name, items.len()))?,
            _ => return Err("no value slot".into()),
        };
        let template = match &current {
            Value::Asset(path) => self.materialize_subobject(&path.clone()),
            Value::Object(Some(o)) => Some(*o),
            _ => None,
        }
        .filter(|&t| self.objects.get(t).is_some_and(|o| o.is_some()))
        .ok_or_else(|| format!("{} holds no object ({current}) whose values a table could change", p.name))?;
        let (sub_class, props, outer) = self.objects[template].as_ref().map(|o| (o.class, o.props.clone(), o.outer.clone())).ok_or("no object")?;
        let name = self.make_unique_name(sub_class, &outer);
        self.object_paths.insert(format!("{outer}.{name}").to_lowercase());
        self.add_name(&name);
        self.objects.push(Some(crate::types::Object { class: sub_class, props, state: None, state_class: None, name, outer }));
        let copy = self.objects.len() - 1;
        self.set_intrinsics(copy);
        let outer_prop = self.prop(template, "Outer");
        if self.prop_slot(copy, "Outer").is_ok() {
            self.set_prop(copy, "Outer", outer_prop);
        }
        let mut sub = self.objects[copy].as_mut().map(|o| std::mem::take(&mut o.props)).ok_or("no object")?;
        let result = sub.with_slice(|v| {
            for (key, fv) in fields {
                let (field, i) = match key.split_once('[') {
                    Some((n, rest)) => (n, Some(rest.strip_suffix(']').and_then(|x| x.parse::<usize>().ok()).ok_or_else(|| format!("\"{key}\" is not a property name (use Name or \"Name[2]\")"))?)),
                    None => (key.as_str(), None),
                };
                self.mod_set_value(sub_class, v, field, i, fv).map_err(|e| format!("{key}: {e}"))?;
            }
            Ok::<(), String>(())
        });
        if let Some(o) = self.objects[copy].as_mut() {
            o.props = sub;
        }
        result?;
        match (index, &mut values[slot]) {
            (Some(i), Value::Array(items)) => items[i] = Value::Object(Some(copy)),
            _ => values[slot] = Value::Object(Some(copy)),
        }
        Ok(())
    }

    /// ImportText form of `v` for a property of type `ty`; `delimited` inside a struct or an array.
    fn mod_text(&mut self, ty: &Type, enumeration: Option<&str>, v: &ModValue, delimited: bool) -> Result<String, String> {
        let wrong = || Err(format!("expects {}, got {v}", type_name(ty)));
        match (ty, v) {
            (Type::Int, ModValue::Int(i)) => i32::try_from(*i).map(|i| i.to_string()).map_err(|_| format!("{i} does not fit in a 32-bit whole number")),
            (Type::Float, ModValue::Int(i)) => Ok(i.to_string()),
            (Type::Float, ModValue::Float(f)) if f.is_finite() => Ok(format!("{f}")),
            (Type::Bool, ModValue::Bool(b)) => Ok(if *b { "True" } else { "False" }.to_string()),
            (Type::Byte, ModValue::Int(i)) if (0..=255).contains(i) => Ok(i.to_string()),
            (Type::Byte, ModValue::Int(i)) => Err(format!("{i} is out of range (0 to 255)")),
            (Type::Byte, ModValue::Str(s)) => {
                let names = enumeration.and_then(|e| self.enum_names(e));
                match names {
                    Some(names) if names.iter().any(|n| n.eq_ignore_ascii_case(s)) => Ok(s.clone()),
                    Some(names) => Err(format!("unknown value {s:?}, expected one of: {}", names.join(", "))),
                    None => Err(format!("expects a number from 0 to 255, got {v}")),
                }
            }
            (Type::Str, ModValue::Str(s)) if !delimited => Ok(s.clone()),
            (Type::Str | Type::Name, ModValue::Str(s)) => Ok(quote(s)),
            (Type::Object(_) | Type::Class(_), ModValue::Str(s)) => Ok(s.clone()),
            (Type::Struct(name), ModValue::Table(fields)) => {
                let defs = self.struct_def(name).map(|s| s.fields.clone()).unwrap_or_default();
                let mut parts = Vec::new();
                for (k, fv) in fields {
                    let Some(f) = defs.iter().find(|f| f.name.eq_ignore_ascii_case(k)) else {
                        let known: Vec<&str> = defs.iter().map(|f| f.name.as_str()).collect();
                        return Err(format!("struct {name} has no field {k} (fields: {})", known.join(", ")));
                    };
                    let t = self.mod_text(&f.ty, f.enumeration.as_deref(), fv, true).map_err(|e| format!("field {k}: {e}"))?;
                    parts.push(format!("{}={t}", f.name));
                }
                Ok(format!("({})", parts.join(",")))
            }
            (Type::Array(inner), ModValue::Array(items)) => {
                let mut parts = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    parts.push(self.mod_text(inner, enumeration, item, true).map_err(|e| format!("element {i}: {e}"))?);
                }
                Ok(format!("({})", parts.join(",")))
            }
            (Type::Delegate | Type::Unknown, _) => Err("this kind of property cannot be changed by mods".into()),
            _ => wrong(),
        }
    }
}

/// Quoted for ReadToken: `"` and `\` escaped (`\22`, `\\`).
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\22"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn type_name(ty: &Type) -> String {
    match ty {
        Type::Byte => "a byte (0 to 255, or an enum value name)".into(),
        Type::Int => "a whole number".into(),
        Type::Bool => "true or false".into(),
        Type::Float => "a number".into(),
        Type::Name => "a name (string)".into(),
        Type::Str => "a string".into(),
        Type::Object(c) => format!("a reference to a {c} (string such as \"Package.Name\")"),
        Type::Class(c) => format!("a class of {c} (string such as \"Package.Class\")"),
        Type::Struct(s) => format!("a {s} struct (inline table such as {{ X = 1.0 }})"),
        Type::Array(inner) => format!("an array of {}", type_name(inner)),
        Type::Delegate => "a delegate".into(),
        Type::Unknown => "an unknown type".into(),
    }
}

/// Plain-English meaning of common weapon and ammo properties (Weapon, Ammunition, Ammo and
/// XIIIBulletsAmmo). Blank when not certain.
fn description(prop: &str) -> &'static str {
    match prop.to_ascii_lowercase().as_str() {
        "reloadcount" => "Shots before a reload (magazine size); 0 = no reloading.",
        "altreloadcount" => "Same as ReloadCount for the alternate ammo.",
        "pickupammocount" => "Ammo inside the weapon when it is picked up.",
        "shottime" => "Seconds an AI-controlled character waits between two shots.",
        "traceaccuracy" => "How precise the shots are, as a percentage of TraceDist.",
        "fvaraccuracy" => "Accuracy that starts low on the first shot and reaches TraceAccuracy after five shots.",
        "fireoffset" => "Where shots start, relative to the first-person eye position.",
        "busesilencer" => "Whether a silencer can be fitted to this weapon.",
        "ammoname" => "Ammo class used by the weapon.",
        "maxammo" => "Maximum ammo that can be carried.",
        "ammoamount" => "Amount of ammo (current amount for an Ammunition, amount given for an Ammo pickup).",
        "srdamages" => "Damage at short range (ShortRange).",
        "lrdamages" => "Damage at long range (LongRange).",
        "shortrange" => "Distance up to which SRDamages applies; the damage decreases beyond it.",
        "longrange" => "Distance up to which LRDamages applies; the damage decreases beyond it.",
        "binfiniteammo" => "Ammo amount reset to its default after each hit (never runs out).",
        "itemname" => "Name shown in the game.",
        // Characters (Pawn, BaseSoldier, GenNMI).
        "health" => "Hit points; a normal full health is 100.",
        "skill" => "Level of the character, 1 to 5.",
        "bboss" => "Marks a boss: when targeted, its name and a health bar are shown.",
        "sightradius" => "How far the character can see.",
        "peripheralvision" => "Width of the field of view, given as the cosine of its edge angle.",
        "hearingthreshold" => "How far away a noise of loudness 1.0 is still heard.",
        "alertness" => "-1 to 1: reaction to stimuli in some states.",
        "groundspeed" => "Top running speed on the ground.",
        "waterspeed" => "Top speed when swimming.",
        "airspeed" => "Top speed when flying.",
        "ladderspeed" => "Speed when climbing a ladder.",
        "accelrate" => "Highest acceleration.",
        "jumpz" => "Vertical speed of a jump.",
        "walkingpct" => "Walking speed, as a fraction of GroundSpeed.",
        "crouchingpct" => "Crouching speed, as a fraction of GroundSpeed.",
        "maxfallspeed" => "Fastest landing that causes no damage; it also limits which paths the AI takes.",
        "agressivite" => "Aggressiveness, -1 to 1.",
        "tempsvisee" => "Aiming time before shooting.",
        "tempsidentification" => "Maximum time (seconds) to identify an enemy at the edge of sight.",
        "distanceattaque" => "Attack distance.",
        "tempsrecherchenmi" => "Time before going back to normal activity after believing it saw something.",
        "pourcerrance" => "Wandering, as a percentage of the patrol.",
        "strategicpointattraction" => "0 to 100: how much the character uses strategic points.",
        "probadeclenchealarme" => "Probability of setting off the nearest alarm.",
        "tempspasvu" => "Time without seeing the enemy before hiding again.",
        "balerte" => "Starts alerted (can also be set by a trigger).",
        "brappliquesialerte" => "When it hears an alarm: comes running (true) or takes cover.",
        "order" => "Name of the starting state.",
        "pv_nmi" => "Health given to the enemies the generator creates.",
        _ => "",
    }
}

/// Engine-held properties that differ for every object (not settable by a mod).
fn intrinsic(name: &str) -> bool {
    ["Name", "Class", "Outer", "ObjectFlags", "ObjectInternal"].iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Markdown table cell: `|` escaped, long values cut.
fn cell(s: &str) -> String {
    let s = s.replace('|', "\\|").replace('\n', " ");
    if s.chars().count() > 120 {
        format!("{}…", s.chars().take(120).collect::<String>())
    } else {
        s
    }
}

impl Vm {
    /// Readable default value: objects as Outer.Name, classes as Package.Class.
    fn reference_value(&self, v: &Value) -> String {
        match v {
            Value::Object(Some(o)) => self.objects.get(*o).and_then(|x| x.as_ref()).map_or(format!("#{o}"), |x| format!("{}.{}", x.outer, x.name)),
            Value::Object(None) | Value::Class(None) => "None".into(),
            Value::Class(Some(c)) => self.class_path(*c),
            Value::Array(items) => format!("({})", items.iter().map(|x| self.reference_value(x)).collect::<Vec<_>>().join(", ")),
            Value::Float(f) => format!("{f:?}"),
            Value::Vector([x, y, z]) => format!("{{ X = {x:?}, Y = {y:?}, Z = {z:?} }}"),
            Value::Rotator([p, y, r]) => format!("{{ Pitch = {p}, Yaw = {y}, Roll = {r} }}"),
            other => format!("{other}"),
        }
    }

    fn reference_type(&self, p: &crate::types::PropDef) -> String {
        fn t(ty: &Type, e: Option<&str>) -> String {
            match ty {
                Type::Byte => match e {
                    Some(e) => format!("byte (enum {})", e.rsplit('.').next().unwrap_or(e)),
                    None => "byte".into(),
                },
                Type::Int => "int".into(),
                Type::Bool => "bool".into(),
                Type::Float => "float".into(),
                Type::Name => "name".into(),
                Type::Str => "string".into(),
                Type::Object(c) => format!("object ({c})"),
                Type::Class(c) => format!("class ({c})"),
                Type::Struct(s) => format!("struct {}", s.rsplit('.').next().unwrap_or(s)),
                Type::Array(inner) => format!("array of {}", t(inner, e)),
                Type::Delegate => "delegate".into(),
                Type::Unknown => "?".into(),
            }
        }
        let base = t(&p.ty, p.enumeration.as_deref());
        if p.dim > 1 {
            format!("{base} [{}]", p.dim)
        } else {
            base
        }
    }

    fn reference_row(&mut self, p: &crate::types::PropDef, v: &Value) -> String {
        let mut shown = self.reference_value(v);
        // Enum value: its name too (the name is what a mod writes).
        if let (Value::Byte(b), Some(e)) = (v, p.enumeration.as_deref()) {
            if let Some(name) = self.enum_names(e).and_then(|n| n.get(*b as usize).cloned()) {
                shown = format!("{name} ({b})");
            }
        }
        format!("| {} | {} | {} | {} |\n", p.name, self.reference_type(p), cell(&shown), description(&p.name))
    }

    /// Tables of the properties declared by each class of `bases` (not by its parent), with
    /// their type and their default in `sample` (or in the base class itself when None).
    fn reference_bases(&mut self, out: &mut String, bases: &[ClassId], sample: Option<ClassId>) {
        for &b in bases {
            let props = self.classes[b].props.clone();
            let start = self.classes[b].super_class.map_or(0, |s| self.classes[s].props.len());
            if start >= props.len() {
                continue;
            }
            let shown = sample.unwrap_or(b);
            self.create_cdo(shown);
            let cdo = self.classes[shown].cdo;
            out.push_str(&format!("### {}\n\n| Property | Type | Default (in {}) | Meaning |\n|---|---|---|---|\n", self.class_path(b), self.classes[shown].name));
            for p in props[start..].iter().filter(|p| !intrinsic(&p.name)) {
                let v = self.prop(cdo, &p.name);
                out.push_str(&self.reference_row(p, &v));
            }
            out.push('\n');
        }
    }

    /// One section per class: the values it sets itself (those that differ from its parent).
    fn reference_classes(&mut self, out: &mut String, classes: Vec<ClassId>) {
        self.reference_classes_with(out, classes, false);
    }

    /// The look a character class shows by default (its own value or its parent's): the text
    /// of the "Look" line of characters.md, and true when it is XIII's own model.
    fn character_look(&mut self, c: ClassId) -> (String, bool) {
        self.create_cdo(c);
        let cdo = self.classes[c].cdo;
        let draw = match self.prop(cdo, "DrawType") {
            Value::Byte(b) => b,
            _ => 2,
        };
        let mesh = self.prop(cdo, "Mesh");
        let mesh = self.reference_value(&mesh);
        let is_xiii = mesh.eq_ignore_ascii_case(XIII_MESH);
        match draw {
            0 => ("invisible (`DrawType` is `DT_None`): nothing is drawn".into(), false),
            8 => {
                let sm = self.prop(cdo, "StaticMesh");
                (format!("a solid model (static mesh) `{}`, not an animated character", self.reference_value(&sm)), false)
            }
            _ if mesh == "None" => ("no model: nothing is drawn until a `Mesh` is given".into(), false),
            _ if is_xiii => (format!("**XIII's own model** (`{mesh}`): this class has no look of its own. Placed in the maps, its characters get their `Mesh` from the level designer (except XIII himself in cutscenes); a character a mod makes from it looks like XIII unless the mod gives it a `Mesh` (see [Character models](character-models.md))"), true),
            _ => (format!("`{mesh}`"), false),
        }
    }

    /// `reference_classes`, with the "Look" line of each class when `looks` (characters.md).
    fn reference_classes_with(&mut self, out: &mut String, mut classes: Vec<ClassId>, looks: bool) {
        classes.sort_by_key(|&c| self.class_path(c).to_lowercase());
        for c in classes {
            self.create_cdo(c);
            let cdo = self.classes[c].cdo;
            let parent = self.classes[c].super_class;
            let parent_cdo = parent.map(|s| {
                self.create_cdo(s);
                self.classes[s].cdo
            });
            let props = self.classes[c].props.clone();
            let mut rows = String::new();
            for p in props.iter().filter(|p| !intrinsic(&p.name)) {
                let v = self.prop(cdo, &p.name);
                if parent_cdo.is_some_and(|pc| self.prop(pc, &p.name) == v) {
                    continue;
                }
                rows.push_str(&self.reference_row(p, &v));
            }
            let parent_name = parent.map(|s| self.class_path(s)).unwrap_or_default();
            out.push_str(&format!("### {}\n\nParent: {parent_name}\n\n", self.class_path(c)));
            if looks {
                let (look, _) = self.character_look(c);
                out.push_str(&format!("Look by default: {look}.\n\n"));
            }
            if rows.is_empty() {
                out.push_str("Sets no value of its own.\n\n");
            } else {
                out.push_str("| Property | Type | Value | Meaning |\n|---|---|---|---|\n");
                out.push_str(&rows);
                out.push('\n');
            }
        }
    }

    /// Chain of classes from `top` (included) down to `class`.
    fn chain_from(&self, top: ClassId, class: ClassId) -> Vec<ClassId> {
        let mut chain = Vec::new();
        let mut k = Some(class);
        while let Some(c) = k {
            chain.push(c);
            if c == top {
                break;
            }
            k = self.classes[c].super_class;
        }
        chain.reverse();
        chain
    }

    /// Subclasses of `root` (not `root` itself) whose package is not in `skip`.
    fn subclasses(&self, root: ClassId, skip: &[&str]) -> Vec<ClassId> {
        (0..self.classes.len())
            .filter(|&c| c != root && self.is_child_of(c, root))
            .filter(|&c| !skip.iter().any(|s| self.packages[self.classes[c].package].name.eq_ignore_ascii_case(s)))
            .collect()
    }

    /// Markdown reference of the weapon and ammo classes (docs/modding/weapons.md): the
    /// properties declared by the base classes (Inventory down to XIIIWeapon or Ammunition) with
    /// their type and default, then for each class the values it sets itself, as the game loads
    /// them.
    pub fn weapons_reference(&mut self) -> Result<String, String> {
        let weapon = self.class("XIII.XIIIWeapon").ok_or("class XIIIWeapon not found")?;
        let ammo = self.class("Engine.Ammunition").ok_or("class Ammunition not found")?;
        let inventory = self.class("Engine.Inventory").ok_or("class Inventory not found")?;
        let mut out = String::new();
        out.push_str("# Weapon and ammo reference\n\n");
        out.push_str(HEADER);
        out.push_str("Under each class, only the values it sets itself are listed; the others come from its parent\n");
        out.push_str("class. The base class tables list every weapon or ammo property with its type.\n\n");
        out.push_str("**Rate of fire.** The player's rate of fire is not a property: in the `NormalFire` state of\n");
        out.push_str("`XIIIWeapon`, the next shot is allowed when the first-person firing animation ends\n");
        out.push_str("(`AnimEnd`), and `PlayFiring` plays it at rate 1.0. A mod changes it with `FireAnimRate`\n");
        out.push_str("(and the reload and select times with `ReloadAnimRate` and `SelectAnimRate`), see\n");
        out.push_str("[expert/weapons.md](expert/weapons.md#weapon-animation-speeds). `ShotTime` is only used by the AI controllers.\n\n");
        out.push_str("Contents: [Weapon base classes](#weapon-base-classes), [Weapons](#weapons), [Ammo base classes](#ammo-base-classes), [Ammo](#ammo).\n\n");
        for (title, base_title, root) in [("Weapons", "Weapon base classes", weapon), ("Ammo", "Ammo base classes", ammo)] {
            out.push_str(&format!("## {base_title}\n\n"));
            let chain = self.chain_from(inventory, root);
            self.reference_bases(&mut out, &chain, Some(root));
            out.push_str(&format!("## {title}\n\n"));
            // The base classes too (not Inventory): the inherited values they change.
            let mut classes = self.subclasses(root, &[]);
            classes.extend(chain.iter().skip(1));
            self.reference_classes(&mut out, classes);
        }
        Ok(out)
    }

    /// Markdown reference of the character classes (docs/modding/characters.md): the properties
    /// declared by Pawn, XIIIPawn and BaseSoldier and by the enemy generators, then for each
    /// character class of the solo game the values it sets itself, as the game loads them.
    pub fn characters_reference(&mut self) -> Result<String, String> {
        let pawn = self.class("Engine.Pawn").ok_or("class Pawn not found")?;
        let soldier = self.class("XIDPawn.BaseSoldier").ok_or("class BaseSoldier not found")?;
        let mut out = String::new();
        out.push_str("# Character reference\n\n");
        out.push_str(HEADER);
        out.push_str("The base class tables list the character properties with their type and their default in\n");
        out.push_str("that class. Under each character class, only the values it sets itself are listed; the\n");
        out.push_str("others come from its parent class. Multiplayer classes (package XIIIMP) are left out.\n\n");
        out.push_str("Characters placed in a map often carry their own values, saved by the level designer: use\n");
        out.push_str("`apply_to_placed = true` or a `[map.<Map>.class.\"...\"]` table to change them (see\n");
        out.push_str("[expert/characters.md](expert/characters.md#changing-characters)). Enemies created during play by a generator\n");
        out.push_str("(`XIDPawn.GenNMI` and others, last section) take several of their settings from the\n");
        out.push_str("generator placed in the map.\n\n");
        out.push_str("Contents: [Character base classes](#character-base-classes), [Characters](#characters), [AI states](#ai-states), [Enemy generators](#enemy-generators).\n\n");
        out.push_str("## Character base classes\n\n");
        let chain = self.chain_from(pawn, soldier);
        self.reference_bases(&mut out, &chain, None);
        out.push_str("## Characters\n\n");
        // The base classes too (not Pawn): the inherited values they change, such as Health.
        let mut classes = self.subclasses(pawn, &["XIIIMP", "Engine"]);
        classes.retain(|c| !chain.contains(c));
        classes.extend(chain.iter().skip(1));
        // Summary: the look of each class, so that a class name is never taken for a look.
        out.push_str("A class name says what a character does (its AI, its stats), not what it looks like: the look\n");
        out.push_str("is the `Mesh` (an animated model of [content/xiiipersos.md](content/xiiipersos.md), listed\n");
        out.push_str("with the characters who wear it in the maps in [character-models.md](character-models.md)).\n");
        out.push_str("Several classes show **XIII's own model** by default: the level designer gives their\n");
        out.push_str("characters in the maps another `Mesh` (those that keep it are XIII himself in cutscenes), so\n");
        out.push_str("a character a mod makes from one of them looks like XIII unless the mod gives it a `Mesh`\n");
        out.push_str("(the log warns about it).\n\n");
        out.push_str("| Class | Look by default | In-game name (`PawnName`) |\n|---|---|---|\n");
        let mut sorted = classes.clone();
        sorted.sort_by_key(|&c| self.class_path(c).to_lowercase());
        for &c in &sorted {
            let (look, is_xiii) = self.character_look(c);
            let path = self.class_path(c);
            let mut short = if is_xiii && path.eq_ignore_ascii_case("XIII.XIIIPlayerPawn") {
                format!("**XIII** (`{XIII_MESH}`), the player")
            } else if is_xiii {
                format!("**XIII** (`{XIII_MESH}`): give it a `Mesh`")
            } else {
                look.split(':').next().unwrap_or_default().to_string()
            };
            if let Some((_, note)) = CLASS_NOTES.iter().find(|(k, _)| k.eq_ignore_ascii_case(&path)) {
                short = format!("{short}: {note}");
            }
            let cdo = self.classes[c].cdo;
            let name = match self.prop(cdo, "PawnName") {
                Value::Str(s) if !s.is_empty() => s,
                _ => "-".into(),
            };
            out.push_str(&format!("| [{path}](#{}) | {short} | {} |\n", path.to_lowercase().replace('.', ""), cell(&name)));
        }
        out.push('\n');
        self.reference_classes_with(&mut out, classes, true);
        // States of the AI controllers of these classes, for allowed_states and state looks.
        let mut controllers: Vec<usize> = Vec::new();
        let mut all = self.subclasses(pawn, &["XIIIMP", "Engine"]);
        all.push(pawn);
        for c in all {
            self.create_cdo(c);
            if let Value::Class(Some(k)) = self.prop(self.classes[c].cdo, "ControllerClass") {
                if !controllers.contains(&k) {
                    controllers.push(k);
                }
            }
        }
        controllers.sort_by_key(|&k| self.class_path(k).to_lowercase());
        out.push_str("## AI states\n\n");
        out.push_str("States of the AI controller of each character class (`ControllerClass`), with the states\n");
        out.push_str("inherited from its parent classes: the names to use in `allowed_states` and\n");
        out.push_str("`[character.<Name>.state.<State>]`. A character's animations are those of its mesh; a wrong\n");
        out.push_str("animation name prints the list in the log.\n\n");
        for k in controllers {
            let users: Vec<String> = (0..self.classes.len())
                .filter(|&c| self.classes[c].cdo != usize::MAX && matches!(self.prop(self.classes[c].cdo, "ControllerClass"), Value::Class(Some(x)) if x == k))
                .map(|c| self.class_path(c))
                .collect();
            out.push_str(&format!("### {}\n\nUsed by: {}\n\nStates: {}\n\n", self.class_path(k), users.join(", "), self.mod_state_names(k).join(", ")));
        }
        if let Some(gen) = self.class("XIDPawn.gennmi") {
            out.push_str("## Enemy generators\n\n");
            out.push_str("Subclasses of `XIDPawn.GenFRD`, placed in the maps: the generators (`gennmi` and its\n");
            out.push_str("subclasses) create enemies during play, the other ones are actors the AI uses (patrols,\n");
            out.push_str("alerts, targets). These are their own settings.\n\n");
            let top = self.class("XIDPawn.GenFRD").unwrap_or(gen);
            let people = self.chain_from(top, gen);
            self.reference_bases(&mut out, &people, None);
            let mut more = self.subclasses(top, &["XIIIMP"]);
            more.retain(|c| !people.contains(c));
            more.extend(people.iter().skip(1));
            self.reference_classes(&mut out, more);
        }
        Ok(out)
    }
}

/// XIII's own animated model, the default `Mesh` of several character classes.
const XIII_MESH: &str = "XIIIPersos.XIIIM";

/// Classes whose name suggests another look or role, checked in the game (characters.md).
const CLASS_NOTES: &[(&str, &str)] = &[
    ("XIDPawn.FBI_Helico01a", "a uniformed guard, not an FBI agent"),
    ("XIII.MitraillGuard", "the unseen gunner of a mounted machine gun, not a guard to place"),
];

/// Opening paragraph of the generated references.
const HEADER: &str = "All the names a mod can use: [names/](names/README.md).\n\nGenerated by `oxiii mods-reference <game folder> docs/modding` (oxiii built with `--features mods`)\n\
from the original game's scripts: the values are the ones the game loads, without mods. Use these\n\
class and property names in `[class.\"Package.Class\"]` tables (see [README.md](README.md)).\n\n";
