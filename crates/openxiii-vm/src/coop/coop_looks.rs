//! Improved build only: the characters the co-op players chose on the character screen
//! (character_screen.rs), in the game.
//!
//! - The body: each co-op player's pawn takes the model and skin of their choice (seen by the
//!   other player; a player's own body is not drawn in their view, as in the game).
//! - The first-person arms: the weapons and objects a player holds are drawn with their
//!   character's arms. The arms are made from the player's own game files by the recipes of
//!   coop-arms/*.toml (openxiii_assets::mod_arms), built once when the choice is confirmed,
//!   and kept under their own names ("Mod.Jones.FpsBerrettaM"): unlike a mod with the same recipe, which
//!   would replace XIII's arms for the whole game, here only the meshes owned by that player are drawn
//!   with them ([`Vm::arms_mesh_override`], read by the game window). What the game does with
//!   those meshes (animations, sounds, timing) is unchanged.
//! - Over the network, a joining player sends their choice with their LOGIN
//!   ([`CHARACTER_OPTION`]); the host gives it to their pawn.
//!
//! Outside a co-op session nothing here runs: the solo game is the original one.

use crate::character_screen::{Choice, ROSTER};
use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// LOGIN URL option of a network co-op client: the character it chose ("OXIIIChar=Jones.2").
pub const CHARACTER_OPTION: &str = "OXIIIChar";

/// The arms recipes of the roster (name of the `[arms.<Name>]` table, its file in coop-arms).
#[cfg(feature = "mods")]
const RECIPES: &[(&str, &str)] = &[
    ("Jones", include_str!("../../coop-arms/jones.toml")),
    ("Carrington", include_str!("../../coop-arms/carrington.toml")),
    ("Mongoose", include_str!("../../coop-arms/mangouste.toml")),
    ("Kim", include_str!("../../coop-arms/kim.toml")),
    ("Amos", include_str!("../../coop-arms/amos.toml")),
    ("AmosCream", include_str!("../../coop-arms/amos-cream.toml")),
    ("McCall", include_str!("../../coop-arms/mccall.toml")),
    ("Sheridan", include_str!("../../coop-arms/sheridan.toml")),
    ("SheridanGrey", include_str!("../../coop-arms/sheridan-grey.toml")),
    ("Standwell", include_str!("../../coop-arms/standwell.toml")),
    ("StandwellRobe", include_str!("../../coop-arms/standwell-robe.toml")),
    ("Admiral", include_str!("../../coop-arms/admiral.toml")),
    ("Spads", include_str!("../../coop-arms/spads.toml")),
    ("SpadsJungle", include_str!("../../coop-arms/spads-jungle.toml")),
    ("GI", include_str!("../../coop-arms/gi.toml")),
    ("FBI", include_str!("../../coop-arms/fbi.toml")),
    ("FBI2", include_str!("../../coop-arms/fbi-bare.toml")),
    ("Killer1", include_str!("../../coop-arms/killer1.toml")),
    ("Killer2", include_str!("../../coop-arms/killer2.toml")),
    ("Killer3", include_str!("../../coop-arms/killer3.toml")),
    ("Killer5", include_str!("../../coop-arms/killer5.toml")),
    ("Conspirator", include_str!("../../coop-arms/conjure.toml")),
    ("Policeman", include_str!("../../coop-arms/police.toml")),
    ("ThinPoliceman", include_str!("../../coop-arms/police-thin.toml")),
];

/// Arms recipes of a one-armed character (Amos): in first person, the left fist's punch is
/// played by the right hand (as his arms mod does for the whole game with
/// `[class."XIII.Fists"] FirstPersonAnimations`, here for that player only).
const ONE_HANDED_ARMS: &[&str] = &["Amos", "AmosCream"];

/// First-person sequences of the left hand and the right hand's that replace them.
const ONE_HANDED_SWAPS: &[(&str, &str)] = &[("FireL", "fireR")];

/// Suffix of the path of an animation that carries those replacements.
const ONE_HANDED_SUFFIX: &str = "#onehanded";

/// `base` with the one-handed replacements put first under the left hand's names (a lookup takes
/// the first sequence with a name); `None` when it has none of them.
fn one_handed_set(base: &crate::anim::AnimSet) -> Option<crate::anim::AnimSet> {
    let mut seqs = Vec::new();
    let mut moves = Vec::new();
    for &(from, to) in ONE_HANDED_SWAPS {
        let (Some(_), Some(i)) = (base.seq(from), base.seq(to)) else { continue };
        let mut seq = base.seqs[i].clone();
        seq.name = from.to_string();
        seqs.push(seq);
        moves.push(base.moves.get(i)?.clone());
    }
    if seqs.is_empty() {
        return None;
    }
    seqs.extend(base.seqs.iter().cloned());
    moves.extend(base.moves.iter().cloned());
    Some(crate::anim::AnimSet { path: format!("{}{ONE_HANDED_SUFFIX}", base.path), seqs, bones: base.bones.clone(), moves })
}

/// A first-person mesh of the game, whose arms a recipe remakes.
/// The arms recipe of the player holding `a`: the first of its owner, owner's owner and one
/// more that is a player's pawn with arms.
fn holder_arms(owner: impl Fn(ObjId) -> Option<ObjId>, a: ObjId, arms_of: impl Fn(ObjId) -> Option<&'static str>) -> Option<&'static str> {
    let mut o = owner(a);
    for _ in 0..3 {
        let p = o?;
        if let Some(r) = arms_of(p) {
            return Some(r);
        }
        o = owner(p);
    }
    None
}

/// The arms mesh of recipe `recipe` for first-person mesh `mesh` ("Mod.Jones.FpsBerrettaM"),
/// None for a mesh that is not first person.
fn arms_mesh_name(recipe: &str, mesh: &str) -> Option<String> {
    first_person_mesh(mesh).then(|| format!("Mod.{recipe}.{}", mesh.rsplit('.').next().unwrap_or(mesh)))
}

fn first_person_mesh(mesh: &str) -> bool {
    let m = mesh.as_bytes();
    let starts = |p: &str| m.len() > p.len() && m[..p.len()].eq_ignore_ascii_case(p.as_bytes());
    starts("XIIIArmes.") || starts("XIIIDeco.")
}

/// Arms of a recipe made on another thread: the result once finished.
#[cfg(feature = "mods")]
type ArmsSlot = std::sync::Arc<(std::sync::Mutex<Option<Result<(String, openxiii_assets::mod_arms::Built), String>>>, std::sync::Condvar)>;

/// Arms being made (or made) in the background, by lower-case recipe name.
#[cfg(feature = "mods")]
static PREFETCH: std::sync::Mutex<Vec<(String, ArmsSlot)>> = std::sync::Mutex::new(Vec::new());

/// Builds the arms of recipe `name` from `library` (its mesh name and what was made).
#[cfg(feature = "mods")]
fn build_arms(library: &mut openxiii_package::library::Library, name: &str) -> Result<(String, openxiii_assets::mod_arms::Built), String> {
    let text = RECIPES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, t)| *t).ok_or_else(|| format!("no recipe {name}"))?;
    let file = openxiii_mods::defaults::parse_mod_file(text, std::path::Path::new("mod.toml"), name);
    let arms = file.arms.iter().find(|a| a.name.eq_ignore_ascii_case(name)).ok_or_else(|| format!("recipe {name} unreadable"))?;
    let recipe = crate::mod_setup::arms_recipe(arms);
    let mesh_name = arms.mesh_name();
    let built = openxiii_assets::mod_arms::build(library, &mesh_name, &recipe)?;
    Ok((mesh_name, built))
}

/// Starts making the arms of recipe `name` on another thread (the character screen, when a
/// player confirms a character), so that starting the game does not hold it: in memory only,
/// nothing is written on the disk.
#[cfg(feature = "mods")]
pub(crate) fn prefetch_arms(vm: &Vm, name: &str) {
    if openxiii_assets::mod_skeletal::get(&format!("Mod.{name}.FpsBerrettaM")).is_some() {
        return;
    }
    let Some(library) = vm.library.as_ref() else { return };
    let key = name.to_ascii_lowercase();
    let slot: ArmsSlot = {
        let Ok(mut all) = PREFETCH.lock() else { return };
        if all.iter().any(|(k, _)| *k == key) {
            return;
        }
        let slot: ArmsSlot = Default::default();
        all.push((key, slot.clone()));
        slot
    };
    let mut library = library.fresh_copy();
    let name = name.to_string();
    let worker = slot.clone();
    let math = vm.session.math;
    let spawned = std::thread::Builder::new().name("coop-arms".into()).spawn(move || {
        let _math = math.enter();
        openxiii_package::background::lower_this_thread();
        let result = build_arms(&mut library, &name);
        if let Ok(mut r) = worker.0.lock() {
            *r = Some(result);
        }
        worker.1.notify_all();
    });
    if spawned.is_err() {
        if let Ok(mut r) = slot.0.lock() {
            *r = Some(Err("no thread".into()));
        }
    }
}

/// Makes the arms of recipe `name` (once per run): its meshes and textures join the ones made
/// while the game runs. Arms already being made in the background are waited for.
#[cfg(feature = "mods")]
pub(crate) fn ensure_arms(vm: &mut Vm, name: &str) -> Result<(), String> {
    let _math = vm.session.math.enter();
    if openxiii_assets::mod_skeletal::get(&format!("Mod.{name}.FpsBerrettaM")).is_some() {
        return Ok(());
    }
    let key = name.to_ascii_lowercase();
    let slot = PREFETCH.lock().ok().and_then(|mut all| all.iter().position(|(k, _)| *k == key).map(|i| all.remove(i).1));
    let made = match slot {
        Some(slot) => {
            let mut r = slot.0.lock().map_err(|_| "arms thread")?;
            while r.is_none() {
                r = slot.1.wait(r).map_err(|_| "arms thread")?;
            }
            r.take().unwrap_or_else(|| Err("arms thread".into()))
        }
        None => build_arms(vm.library.as_mut().ok_or("no game library")?, name),
    };
    let (mesh_name, built) = made?;
    for (texture, w, h, rgba) in built.textures {
        openxiii_assets::mod_textures::set_runtime(&texture, openxiii_mods::own_textures::mips(w, h, rgba));
    }
    for (from, mesh) in built.meshes {
        let own = format!("{mesh_name}.{}", from.rsplit('.').next().unwrap_or(&from));
        openxiii_assets::mod_skeletal::add_runtime(&own, std::sync::Arc::new(mesh));
    }
    Ok(())
}

impl Vm {
    /// The character of each co-op player of this map: the local players (viewport order) with
    /// the session's choices, and on a network host the remote players with theirs.
    fn coop_player_choices(&mut self) -> Vec<(ObjId, Choice)> {
        let mut out = Vec::new();
        if let Some(chosen) = self.local_play().characters {
            let local = self.screen_players();
            for (i, pc) in local.into_iter().enumerate().take(2) {
                let (character, variant) = chosen[i];
                out.push((pc, Choice { character: character as usize, variant: variant as usize }));
            }
        }
        let remote: Vec<(ObjId, Choice)> = self.improved.character_screen.remote.iter().map(|(&pc, &c)| (pc, c)).collect();
        out.extend(remote.into_iter().filter(|&(pc, _)| self.objects.get(pc).is_some_and(|o| o.is_some())));
        out
    }

    /// Once per level tick in a co-op game: each player's pawn wears their character, and the
    /// first-person meshes they own are listed with their character's arms.
    pub(crate) fn coop_looks_frame(&mut self) {
        if self.improved.character_screen.on {
            return;
        }
        if !self.coop.on && self.improved.character_screen.remote.is_empty() {
            return;
        }
        let choices = self.coop_player_choices();
        if choices.is_empty() {
            return;
        }
        let mut arms_of: Vec<(ObjId, &'static str)> = Vec::new();
        for (pc, choice) in choices {
            let Some(pawn) = self.prop(pc, "Pawn").as_object().filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some())) else { continue };
            let Some(look) = choice.variant_data() else { continue };
            // XIII himself: the game's own look and arms.
            if choice == Choice::default() {
                continue;
            }
            if !matches!(self.native_prop(pawn, Field::Mesh), Value::Asset(m) if m.eq_ignore_ascii_case(look.mesh)) {
                self.coop_wear_character(pawn, choice);
                self.log.push(format!("Log: Co-op: {} plays {}", self.obj_label(pc), choice.code()));
            }
            self.coop_body_animations(pawn);
            #[cfg(feature = "mods")]
            self.coop_voice(pawn, choice);
            if let Some(arms) = look.arms {
                arms_of.push((pawn, arms));
            }
        }
        self.coop_arms_frame(arms_of);
    }

    /// Each co-op player's pawn with their character's arms recipe: the arms of whatever they
    /// hold are resolved when it is drawn (arms_mesh_override), however and whenever it was
    /// made. One-handed arms also get their first-person gestures here.
    fn coop_arms_frame(&mut self, arms_of: Vec<(ObjId, &'static str)>) {
        self.improved.character_screen.arms_by_pawn = arms_of.iter().copied().collect();
        if arms_of.is_empty() || !arms_of.iter().any(|(_, a)| ONE_HANDED_ARMS.contains(a)) {
            return;
        }
        let one_handed: Vec<ObjId> = self
            .actors
            .iter()
            .copied()
            .filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some()))
            .filter(|&a| self.arms_mesh_override(a).is_some() && self.owner_arms(a).is_some_and(|r| ONE_HANDED_ARMS.contains(&r)))
            .collect();
        for a in one_handed {
            self.coop_one_handed_first_person(a);
        }
    }

    /// Entry point for whatever goes with a chosen character's body: a co-op player's pawn
    /// takes `choice`'s model and skin (the look the other players see). Called once per pawn
    /// (again for a pawn that respawns, or whose model a script changed back).
    pub(crate) fn coop_wear_character(&mut self, pawn: ObjId, choice: Choice) {
        let Some(look) = choice.variant_data() else { return };
        // What the body was playing: a new mesh makes a new mesh instance, which starts with no
        // channel at all.
        let playing = self.mesh_insts.get(&pawn).filter(|i| !i.mesh.eq_ignore_ascii_case(look.mesh)).cloned();
        self.set_prop(pawn, "Mesh", Value::Asset(look.mesh.to_string()));
        if let Some(skin) = look.skin {
            self.set_prop(pawn, "Skins", Value::Array(vec![Value::Asset(skin.to_string())]));
        }
        if let Some(playing) = playing {
            self.coop_body_animations(pawn);
            self.coop_keep_playing(pawn, &playing);
        }
    }

    /// A body that just took another mesh goes on with what it was playing on the old one
    /// (`playing`): every channel with its sequence, frame, rates and blend, and the bones the
    /// scripts turned or scaled, found again by name in the new mesh.
    ///
    /// The engine sets a pawn's walking channels up only once (APawn::UpdateMovementAnimation,
    /// bInitializeAnimation): a body given its character after its first tick (the pawn of a
    /// flashback, made by the map's scripts for XIII) would otherwise keep empty channels, and
    /// slide around in a frozen stride.
    fn coop_keep_playing(&mut self, pawn: ObjId, playing: &crate::anim::MeshInstance) {
        if self.mesh_instance(pawn).is_none() {
            return;
        }
        let bone = |vm: &Vm, old: i32| -> i32 {
            let name = usize::try_from(old).ok().and_then(|b| playing.info.bones.get(b));
            match (name, vm.mesh_insts.get(&pawn)) {
                (Some(name), Some(inst)) => crate::skel::match_ref_bone(&inst.info, name),
                _ => old.min(0),
            }
        };
        let mut channels = playing.channels.clone();
        for c in &mut channels {
            c.start_bone = bone(self, c.start_bone).max(0);
            if c.sequence.is_empty() {
                continue;
            }
            // The same gesture in the new mesh's animation, at its own speed there.
            if let Some((index, set, s)) = self.anim_named(pawn, &c.sequence) {
                let seq = &set.seqs[s];
                let orig = seq.rate / seq.num_frames as f32;
                if c.orig_rate != 0.0 && orig != c.orig_rate {
                    let k = orig / c.orig_rate;
                    c.rate *= k;
                    c.old_rate *= k;
                }
                c.orig_rate = orig;
                c.anim_index = index as i32;
                c.notify = !seq.notifies.is_empty();
            }
        }
        let renamed = |vm: &Vm, list: &[crate::skel::Director]| -> Vec<crate::skel::Director> {
            list.iter().cloned().filter_map(|mut d| {
                d.bone = bone(vm, d.bone);
                (d.bone >= 0).then_some(d)
            }).collect()
        };
        let (directors, mesh_dirs) = (renamed(self, &playing.pose.directors), renamed(self, &playing.pose.mesh_dirs));
        let scalers: Vec<crate::skel::Scaler> = playing.pose.scalers.iter().cloned().filter_map(|mut s| {
            s.bone = bone(self, s.bone);
            (s.bone >= 0).then_some(s)
        }).collect();
        let Some(inst) = self.mesh_insts.get_mut(&pawn) else { return };
        inst.channels = channels;
        inst.notify_channel = playing.notify_channel;
        inst.end_channel = playing.end_channel;
        inst.pose.directors = directors;
        inst.pose.mesh_dirs = mesh_dirs;
        inst.pose.scalers = scalers;
        inst.pose.root_lock = playing.pose.root_lock;
        inst.pose.root_locked = playing.pose.root_locked;
    }

    /// A one-armed character's first-person mesh (`a`, owned by that player only): the left
    /// hand's gestures played by the right hand (the punch of the left fist), in this actor's
    /// animation only (other players keep both fists). Cheap and idempotent, as
    /// coop_body_animations.
    fn coop_one_handed_first_person(&mut self, a: ObjId) {
        let Some(inst) = self.mesh_instance(a) else { return };
        let (first, default) = (inst.anims.first().cloned(), inst.info.default_anim.clone());
        if first.as_ref().is_some_and(|s| s.path.ends_with(ONE_HANDED_SUFFIX)) {
            return;
        }
        let base = match first {
            Some(s) => s,
            None => {
                let Some(path) = default else { return };
                let Some(set) = self.anim_set(&path) else { return };
                set
            }
        };
        let key = format!("{}{ONE_HANDED_SUFFIX}", base.path).to_lowercase();
        let set = match self.anim_sets.get(&key) {
            Some(s) => s.clone(),
            None => {
                let s = one_handed_set(&base).map(std::rc::Rc::new);
                self.anim_sets.insert(key, s.clone());
                s
            }
        };
        let Some(set) = set else { return };
        if let Some(inst) = self.mesh_insts.get_mut(&a) {
            match inst.anims.first_mut() {
                Some(first) => *first = set,
                None => inst.anims.push(set),
            }
        }
    }

    /// The mesh drawn for actor `a` instead of its own: a first-person mesh held by a co-op
    /// player, with their character's arms (`held_arms_mesh`), or a co-op player's body fitted
    /// under their vest and helmet (coop_armor.rs).
    pub fn arms_mesh_override(&self, a: ObjId) -> Option<&str> {
        self.held_arms_mesh(a).or_else(|| self.body_mesh_override(a))
    }

    /// The mesh drawn for actor `a` instead of its own: a first-person mesh held by a co-op
    /// player, with their character's arms. The one rule for every held thing (starting
    /// inventory, chosen weapons, pickups, scripted gives, saves, respawns): resolved here, when
    /// it is drawn, from the mesh and the holder's character.
    fn held_arms_mesh(&self, a: ObjId) -> Option<&'static str> {
        let recipe = self.owner_arms(a)?;
        let Value::Asset(mesh) = self.native_prop(a, Field::Mesh) else { return None };
        let key = (recipe, mesh.clone());
        if let Some(&name) = self.improved.character_screen.arms_names.borrow().get(&key) {
            return name;
        }
        let own = arms_mesh_name(recipe, mesh)?;
        #[cfg(feature = "mods")]
        let made = openxiii_assets::mod_skeletal::get(&own).is_some();
        #[cfg(not(feature = "mods"))]
        let made = false;
        let name: Option<&'static str> = made.then(|| &*Box::leak(own.clone().into_boxed_str()));
        if name.is_none() {
            self.improved.character_screen.arms_gaps.borrow_mut().push(format!("{own} (for {mesh})"));
        }
        self.improved.character_screen.arms_names.borrow_mut().insert(key, name);
        name
    }

    /// The arms recipe of the co-op player holding actor `a` (its owner, owner's owner or one
    /// more: weapon, its attachment, the left hand's body), if it is a first-person mesh.
    pub(crate) fn owner_arms(&self, a: ObjId) -> Option<&'static str> {
        let by_pawn = &self.improved.character_screen.arms_by_pawn;
        if by_pawn.is_empty() || by_pawn.contains_key(&a) {
            return None;
        }
        holder_arms(|x| self.native_prop(x, Field::Owner).as_object(), a, |p| by_pawn.get(&p).copied())
    }

    /// Network co-op client: the LOGIN option carrying its player's character.
    pub(crate) fn coop_character_login_option(&self) -> Option<String> {
        let (character, variant) = self.local_play().characters?[0];
        let choice = Choice { character: character as usize, variant: variant as usize };
        (choice.character < ROSTER.len()).then(|| format!("{CHARACTER_OPTION}={}", choice.code()))
    }

    /// Network co-op host: a remote player's character, sent with their LOGIN.
    pub(crate) fn coop_character_from_login(&mut self, pc: ObjId, url: &crate::travel::Url) {
        let key = format!("{CHARACTER_OPTION}=");
        if let Some(choice) = url.get_option(&key).and_then(Choice::from_code) {
            self.improved.character_screen.remote.insert(pc, choice);
            self.log.push(format!("Log: Co-op: remote player plays {}", choice.code()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_person_meshes() {
        assert!(first_person_mesh("XIIIArmes.FpsBerrettaM"));
        assert!(first_person_mesh("xiiideco.FpsChaiseM"));
        assert!(!first_person_mesh("XIIIPersos.JonesM"));
        assert!(!first_person_mesh("XIIIArmes."));
    }

    #[test]
    fn whatever_a_player_holds_takes_their_arms() {
        // Two players (pawns 10 and 20) and what they hold: a weapon (owner: pawn), its
        // attachment (owner: weapon), the left hand's carried body (owner: hand, hand: pawn).
        let owner = |a: ObjId| match a {
            11 | 12 => Some(10),
            13 => Some(11),
            21 => Some(20),
            22 => Some(21),
            23 => Some(22),
            10 => Some(1),
            20 => Some(2),
            _ => None,
        };
        let arms = |p: ObjId| match p {
            10 => Some("Jones"),
            20 => Some("Kim"),
            _ => None,
        };
        for (a, want) in [(11, Some("Jones")), (12, Some("Jones")), (13, Some("Jones")), (21, Some("Kim")), (22, Some("Kim")), (23, Some("Kim")), (99, None), (1, None)] {
            assert_eq!(holder_arms(owner, a, arms), want, "actor {a}");
        }
        // Not a player's: the pawn itself, its controller.
        assert_eq!(holder_arms(owner, 10, arms), None);
        assert_eq!(arms_mesh_name("Kim", "XIIIArmes.FpsBerrettaM").as_deref(), Some("Mod.Kim.FpsBerrettaM"));
        assert_eq!(arms_mesh_name("Kim", "XIIIDeco.FPSMedikitMedM").as_deref(), Some("Mod.Kim.FPSMedikitMedM"));
        assert_eq!(arms_mesh_name("Kim", "XIIIPersos.KimM"), None, "a body, not something held");
    }

    /// Every first-person mesh of the game (weapons and items, in XIIIArmes and XIIIDeco: what a
    /// player can hold, however they got it) has the arms of every roster character, for both
    /// players; the gaps are printed (OXIII_GAME; skipped when it is not set).
    /// Arms started in the background (the character screen) are taken as they are when the
    /// choice is confirmed: the same meshes as made directly (OXIII_GAME; skipped without it).
    #[cfg(feature = "mods")]
    #[test]
    fn arms_made_in_the_background_are_taken_at_confirmation() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| openxiii_package::system_dir(g).exists()) else {
            eprintln!("OXIII_GAME not set: skipped");
            return;
        };
        let mut vm = Vm::new();
        vm.library = Some(openxiii_package::library::Library::new(&game).expect("library"));
        let t = std::time::Instant::now();
        prefetch_arms(&vm, "Kim");
        let started = t.elapsed();
        std::thread::sleep(std::time::Duration::from_millis(600));
        let t = std::time::Instant::now();
        ensure_arms(&mut vm, "Kim").expect("Kim");
        let taken = t.elapsed();
        eprintln!("prefetch started in {:.2} ms, taken at confirmation in {:.2} ms", started.as_secs_f64() * 1000.0, taken.as_secs_f64() * 1000.0);
        assert!(openxiii_assets::mod_skeletal::get("Mod.Kim.FpsBerrettaM").is_some());
        assert!(PREFETCH.lock().unwrap().iter().all(|(k, _)| k != "kim"));
    }

    #[cfg(feature = "mods")]
    #[test]
    fn every_held_mesh_has_every_characters_arms() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| openxiii_package::system_dir(g).exists()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let mut vm = Vm::new();
        vm.library = Some(openxiii_package::library::Library::new(&game).expect("library"));
        let held = {
            let library = vm.library.as_mut().unwrap();
            openxiii_assets::mod_arms::sources(library, &["XIIIArmes.*".to_string(), "XIIIDeco.*".to_string()], &[]).expect("meshes")
        };
        assert!(held.len() > 30, "{} first-person meshes", held.len());
        let mut recipes: Vec<&'static str> = ROSTER.iter().flat_map(|c| c.variants.iter().filter_map(|v| v.arms)).collect();
        recipes.dedup();
        let mut gaps = Vec::new();
        for (i, recipe) in recipes.iter().enumerate() {
            ensure_arms(&mut vm, recipe).unwrap_or_else(|e| panic!("{recipe}: {e}"));
            // Player 1 or 2 (alternately), the mesh held at any depth: the same rule.
            let pawn: ObjId = if i % 2 == 0 { 10 } else { 20 };
            let owner = |a: ObjId| match a {
                100 => Some(pawn),
                101 => Some(100),
                _ => None,
            };
            for mesh in &held {
                for actor in [100, 101] {
                    let got = holder_arms(owner, actor, |p| (p == pawn).then_some(*recipe)).and_then(|r| arms_mesh_name(r, mesh));
                    let name = got.unwrap_or_else(|| panic!("{recipe} {mesh}: no arms name"));
                    if openxiii_assets::mod_skeletal::get(&name).is_none() && actor == 100 {
                        gaps.push(name);
                    }
                }
            }
        }
        if !gaps.is_empty() {
            eprintln!("held meshes without a character's arms ({}): {}", gaps.len(), gaps.join(", "));
        }
        assert!(gaps.is_empty(), "{} held meshes keep XIII's hands", gaps.len());
    }

    #[cfg(feature = "mods")]
    #[test]
    fn every_roster_arms_has_its_recipe() {
        for c in ROSTER {
            for v in c.variants {
                if let Some(name) = v.arms {
                    let text = RECIPES.iter().find(|(n, _)| *n == name).map(|(_, t)| *t).unwrap_or_else(|| panic!("no recipe {name}"));
                    let file = openxiii_mods::defaults::parse_mod_file(text, std::path::Path::new("mod.toml"), name);
                    assert!(file.arms.iter().any(|a| a.name == name), "{name}: table [arms.{name}] in its mod.toml");
                }
            }
        }
    }

    #[test]
    fn the_login_option_carries_the_choice() {
        let mut vm = Vm::default();
        assert_eq!(vm.coop_character_login_option(), None);
        vm.change_local_play(|p| p.characters = Some([(13, 2), (0, 0)]));
        assert_eq!(vm.coop_character_login_option().as_deref(), Some("OXIIIChar=Killer.3"));
        let url = crate::travel::Url::parse(None, "Hual01a?Name=Player?OXIIIChar=Killer.3", crate::travel::TRAVEL_ABSOLUTE);
        vm.coop_character_from_login(7, &url);
        assert_eq!(vm.improved.character_screen.remote.get(&7), Some(&Choice { character: 13, variant: 2 }));
    }

    /// A flashback's pawn is made for XIII by the map's scripts and only gets its player's
    /// character a tick later, once the engine has set its walking channels up (which it does
    /// only once): the body must go on with those channels on its new mesh, and walk.
    /// OXIII_GAME; skipped when it is not set.
    #[test]
    fn a_body_given_its_character_late_still_walks() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| openxiii_package::system_dir(g).exists()) else {
            eprintln!("OXIII_GAME not set: skipped");
            return;
        };
        let mut vm = Vm::new();
        vm.ipdrv.offline = true;
        vm.load_dir(&game.join("System")).unwrap();
        let url = crate::travel::start_url(&game.join("System"), "Plage01");
        vm.url = url.clone();
        vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
        let game_class = vm.game_class_for(&url);
        let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
        vm.begin_play();
        let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
        let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
        let dt = 1.0 / 30.0;
        // The pawn's first tick as XIII: the engine's one-time setup of the walking channels.
        vm.set_prop(pawn, "bPhysicsAnimUpdate", Value::Bool(true));
        vm.pawn_tick_special(pawn, dt);
        assert!(vm.prop(pawn, "bInitializeAnimation").as_bool());
        let setup = vm.mesh_instance(pawn).unwrap().clone();
        assert!(setup.channels.len() > 13 && !setup.channels[4].sequence.is_empty(), "{:?}", vm.anim_channels(pawn));
        let spine = |i: &crate::anim::MeshInstance| crate::skel::match_ref_bone(&i.info, "X Spine");
        assert_eq!(setup.channels[12].start_bone, spine(&setup));
        // Then the player's character.
        let choice = Choice::from_code("Carrington").unwrap();
        vm.coop_wear_character(pawn, choice);
        let worn = vm.mesh_instance(pawn).unwrap().clone();
        assert!(worn.mesh.eq_ignore_ascii_case(choice.variant_data().unwrap().mesh));
        assert_eq!(worn.channels.len(), setup.channels.len());
        for (k, (a, b)) in setup.channels.iter().zip(&worn.channels).enumerate() {
            assert_eq!((&a.sequence, a.notify_ignore, a.looping, a.blend_alpha), (&b.sequence, b.notify_ignore, b.looping, b.blend_alpha), "channel {k}");
            if !b.sequence.is_empty() {
                assert!(worn.anims.get(b.anim_index as usize).is_some_and(|s| s.seq(&b.sequence).is_some()), "channel {k}: {}", b.sequence);
            }
        }
        assert_eq!((worn.channels[12].start_bone, worn.channels[13].start_bone), (spine(&worn), spine(&worn)));
        assert!(spine(&worn) > 0);
        // Walking straight ahead for half a second: the stride plays.
        let ahead = crate::ai_natives::axes(vm.native_prop(pawn, Field::Rotation).as_rotator())[0];
        for _ in 0..15 {
            vm.set_prop(pawn, "Physics", Value::Byte(1));
            vm.set_prop(pawn, "Acceleration", Value::vector(ahead[0] * 2048.0, ahead[1] * 2048.0, 0.0));
            vm.set_prop(pawn, "Velocity", Value::vector(ahead[0] * 400.0, ahead[1] * 400.0, 0.0));
            vm.pawn_tick_special(pawn, dt);
            vm.update_animation(pawn, dt);
        }
        let stride = vm.mesh_instance(pawn).unwrap().channels[4].clone();
        assert!(stride.blend_alpha > 0.9 && stride.frame > 0.2, "stride {} frame {} alpha {}", stride.sequence, stride.frame, stride.blend_alpha);
        assert!(vm.get_frame(pawn).is_some());
    }
}
