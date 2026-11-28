//! Local co-op campaign, extra enemies (see coop.rs; improved build only, off by default).
//!
//! With more than one player the solo maps are easier: each enemy shares its attention between
//! two players. This part adds enemies at the start of a co-op map, `percent` % more per extra
//! player (0: none, the map as it is):
//! - Placed soldiers: an ordinary enemy (class BaseSoldier, Alliance NMI, GameOver 0, not named
//!   by a map script) gets a copy next to it, with the same editable properties (ORDER, patrol
//!   and network numbers, senses, skill, health, Tag, Event…) and the same weapons, never its
//!   story items. With 50 % one soldier out of two, in the map's order.
//! - Generators (gennmi): NbNMIMax, NbNMIActifs and NbNMIGen grow by the same ratio; an
//!   unlimited generator (NbNMIMax -1) is left as it is.
//! - Death events: a soldier and its copies form a group; the group's death Event fires when
//!   its last member dies, never earlier (the name moves to a living member just before the
//!   death's script runs). Generated soldiers are grouped in spawn order.
//!
//! Nothing here runs unless co-op is on and `percent` is not 0. Saves: a checkpoint reloads the
//! map from scratch (only the players' things are saved), the copies are made again then; the
//! soldiers a checkpoint removes (XIIISaveGameTrigger.CleanMap) are gone before the copies are
//! made, and a placed soldier removed later by the story takes its copies with it.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::collections::{HashMap, HashSet};

use crate::types::FuncId;
use crate::value::{NameStruct, ObjId, Value};
use crate::Vm;

/// Classes whose own properties name soldiers the story needs by object or by name (counted
/// kills, scripted scenes: escort, sleeping or hidden soldiers, scripted killings, camera
/// windows on a soldier): those soldiers, and the generators they name, are left as they are.
/// The soldiers a fight sequencer switches (XIIIAttackSequencer) or a checkpoint removes are
/// ordinary fights and stay eligible. The speakers of a scripted dialogue (DialogueManager,
/// by object; a generator speaker talks through its last soldier) are never copied either, so a
/// conversation is never held next to a silent double and never moves to an extra.
const SCRIPT_REFERRERS: [&str; 25] = [
    "KillCounterTrigger", "SoldierTouchAndKill", "Patrouille", "BossHealthBar", "EscortManager", "AwakeSoldiers",
    "PutSoldiersToSleep", "CineKiller", "CineMalletteSM", "CorpseDetectorTrigger", "GrabbedCorpseTrigger", "DetectionVolume",
    "TriggerPetitCachottier", "ZigouillateurTrigger", "ZigouillateurTriggerBis", "ZigouillateurVolume", "CWndBackShotTrigger",
    "CWndFocusTrigger", "CamViewTrigger", "MultiViewTrigger", "KelloAmbush", "NastyHelico", "GenFlash", "RoofGrapnleDemonstrator",
    "DialogueManager",
];
/// Scenes the rules above do not recognise: (map, Tag or name of a soldier or a generator, or
/// a generator's Bases_Tag), never copied or scaled. Lower case.
const EXCLUDED: &[(&str, &str)] = &[];
/// Rings tried for a copy around its original, nearest first: (distance in units, places on
/// the ring). Each copy starts its rings at another angle (golden angle steps) so that copies
/// spread round their original instead of piling on one side.
const RINGS: [(f32, usize); 5] = [(90.0, 6), (140.0, 8), (200.0, 10), (270.0, 12), (350.0, 14)];
const GOLDEN_ANGLE: f32 = 2.399_963;
/// Floor of a place: at most this much above or under the original's floor (one step of
/// walking; a ledge, a lower level or a raised platform is refused).
const MAX_STEP: f32 = 36.0;
/// How far above the original's centre the floor search starts.
const GROUND_ABOVE: f32 = 50.0;
/// Looks of enemies this close to an original (or in its alert network) are preferred.
const NEAR_LOOK: f32 = 3000.0;
/// A copy with no free ground next to its original may stand on a path node this close.
const NODE_DISTANCE: f32 = 400.0;
/// Room kept between two pawns placed side by side.
const PAWN_GAP: f32 = 4.0;
/// Properties of an original set after the spawn by their own natives, not copied raw.
const COLLISION_PROPS: [&str; 5] = ["bCollideActors", "bBlockActors", "bBlockPlayers", "CollisionRadius", "CollisionHeight"];
/// Properties a copy never takes from its original.
const NEVER_COPIED: [&str; 6] = ["Location", "Rotation", "Owner", "Base", "Tag", "InitialInventory"];

/// Extra enemies state of a Vm (inside `Coop`).
#[derive(Debug, Default, Clone)]
pub struct CoopEnemies {
    /// Setting: extra enemies in percent per extra player (0, 50 or 100).
    pub percent: u32,
    /// Original whose properties the next spawned actor takes (spawn_actor).
    pub(crate) template: Option<ObjId>,
    /// Groups of soldiers that share one death Event.
    pub(crate) groups: Vec<Group>,
    /// Group of each soldier in a group.
    pub(crate) member: HashMap<ObjId, usize>,
    /// Scaled generators: ratio and soldiers generated so far.
    pub(crate) generators: HashMap<ObjId, Generator>,
    /// Group of each (generator, group number).
    pub(crate) generated_groups: HashMap<(ObjId, usize), usize>,
    /// What the start of the map did (for the logs and the tests).
    pub report: Report,
    /// Looks of the map's eligible enemies, an extra's look is drawn from (see Look).
    pub(crate) donors: Vec<Donor>,
    /// Look the next spawned copy takes (spawn_actor).
    pub(crate) template_look: Option<Look>,
    /// Copy being spawned: what it is born touching gets no Touch event (physics.rs
    /// touch_add). A placed soldier standing in a volume comes with the map in contact with it
    /// and gets no Touch at the start either; Touch is only for what it walks into later.
    pub(crate) quiet_spawn: Option<ObjId>,
    /// Tests only: copies spawned with their Touch events (the order before quiet_spawn).
    pub(crate) loud_spawns: bool,
    /// Scaled generators: their own Mesh, Skins and CodeMesh (lent to an extra while gennmi
    /// dresses it, then given back), and the looks their extras got.
    pub(crate) generator_looks: HashMap<ObjId, [Value; 3]>,
    pub(crate) generator_taken: HashMap<ObjId, Vec<String>>,
    /// Copies tried so far on this map (each starts its rings at another angle).
    pub(crate) copies_tried: usize,
    /// Places checked once after a few seconds (debugging).
    pub(crate) checked: bool,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Group {
    /// The original first, then its copies (or the generated soldiers in spawn order).
    pub(crate) members: Vec<ObjId>,
    /// Member already through its death (same order).
    pub(crate) dead: Vec<bool>,
    /// Placed soldier group (its copies go with the original if the story removes it).
    pub(crate) placed: bool,
    /// Where each copy was put (placed groups; `None` for the original).
    pub(crate) places: Vec<Option<[f32; 3]>>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Generator {
    pub(crate) ratio: f64,
    pub(crate) spawned: usize,
}

/// What an enemy looks and sounds like: its mesh, skins, voice family (CodeMesh) and voice
/// (NumeroTimbre; None: the generator draws it as usual). Nothing else of the donor is taken:
/// an extra keeps its original's orders, senses, health, inventory (weapons, helmet, vest) and
/// animation links.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Look {
    pub(crate) mesh: Value,
    pub(crate) skins: Vec<Value>,
    pub(crate) code_mesh: i32,
    pub(crate) timbre: Option<u8>,
}

impl Look {
    pub(crate) fn key(&self) -> String {
        let skins: Vec<String> = self.skins.iter().map(|s| s.to_string()).collect();
        format!("{} [{}]", self.mesh, skins.join(","))
    }
}

/// An eligible enemy of the map as a source of looks.
#[derive(Debug, Clone)]
pub(crate) struct Donor {
    pub(crate) look: Look,
    /// Possible voices of a generator ("123;": timbres 1 to 3), for an extra that takes its look.
    pub(crate) timbres: String,
    pub(crate) at: [f32; 3],
    pub(crate) network: i32,
    pub(crate) alliance: String,
    /// Starts with a helmet (Casque in its inventory): the helmet shows on the outfit, so an
    /// extra only takes looks of enemies with the same headgear as its original.
    pub(crate) helmet: bool,
    /// Animation sets beyond the common male set (MigA): a role (agent, sheriff, guard…); such
    /// an enemy neither lends its look nor gets another one.
    pub(crate) role: bool,
}

/// Small generator of the looks, apart from the game's random stream (never drawn from): the
/// same map, enemy and extra always give the same look, on every machine.
pub(crate) struct LookRng(u64);

impl LookRng {
    pub(crate) fn new(map: &str, who: &str, extra: usize) -> LookRng {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let bytes = map.to_ascii_lowercase().into_bytes().into_iter().chain([0u8]).chain(who.to_ascii_lowercase().into_bytes()).chain([0u8]).chain((extra as u64).to_le_bytes());
        for b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        LookRng(h)
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// The donor whose look an extra of `original` takes, or None (it keeps its original's look):
/// same alliance, same headgear, no role look on either side, another look than the original's;
/// looks of enemies near the original or in its alert network first, then the whole map; looks
/// not yet given to the original's other extras first.
pub(crate) fn choose_look(original: &Donor, donors: &[Donor], taken: &[String], rng: &mut LookRng) -> Option<usize> {
    if original.role {
        return None;
    }
    let own = original.look.key();
    let fits = |d: &Donor| !d.role && d.helmet == original.helmet && d.alliance.eq_ignore_ascii_case(&original.alliance) && d.look.key() != own;
    let near = |d: &Donor| {
        let dist = ((d.at[0] - original.at[0]).powi(2) + (d.at[1] - original.at[1]).powi(2) + (d.at[2] - original.at[2]).powi(2)).sqrt();
        dist <= NEAR_LOOK || (original.network != 0 && d.network == original.network)
    };
    for nearby in [true, false] {
        let mut keys: Vec<String> = donors.iter().filter(|d| fits(d) && (!nearby || near(d))).map(|d| d.look.key()).collect();
        keys.sort();
        keys.dedup();
        if keys.is_empty() {
            continue;
        }
        let fresh: Vec<&String> = keys.iter().filter(|k| !taken.contains(k)).collect();
        let pool: Vec<&String> = if fresh.is_empty() { keys.iter().collect() } else { fresh };
        let key = pool[rng.below(pool.len())].clone();
        return donors.iter().position(|d| fits(d) && d.look.key() == key);
    }
    None
}

/// A voice among a generator's possible ones ("123;" -> timbre 0, 1 or 2), by `rng`.
pub(crate) fn timbre_of(timbres: &str, rng: &mut LookRng) -> Option<u8> {
    let digits: Vec<u8> = timbres.split(';').next().unwrap_or("").bytes().filter(|b| (b'1'..=b'3').contains(b)).map(|b| b - b'1').collect();
    (!digits.is_empty()).then(|| digits[rng.below(digits.len())])
}

/// Counts of the start of a map.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    /// Ordinary enemies placed in the map (eligible or not).
    pub placed_enemies: usize,
    /// Of those, the ones that may be copied.
    pub eligible: usize,
    /// Copies wanted, made, made on a path node, refused for lack of room.
    pub wanted: usize,
    pub made: usize,
    pub on_node: usize,
    pub no_room: usize,
    /// Copies that had a place but could not be made (spawn refused, no AI), by reason.
    pub failed: Vec<(String, usize)>,
    /// Refusals by reason (class, alliance or GameOver, named by a script, not alive, no AI).
    pub refused: Vec<(String, usize)>,
    /// Generators: scaled ones, unlimited ones left alone, soldiers the scaled ones make in
    /// total before and after.
    pub generators: usize,
    pub unlimited_generators: usize,
    pub generated_before: i64,
    pub generated_after: i64,
    /// Generators left alone by a rule (ally, named by a script, excluded scene).
    pub generators_refused: usize,
    /// Each extra's look: "<extra> of <original>: <original's look> -> <its look>".
    pub looks: Vec<String>,
    /// Each soldier and generator looked at: "kept" or the refusal, its name and Tag.
    pub decisions: Vec<String>,
}

/// Why a placed soldier is not copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Not exactly class BaseSoldier (bosses, scripted characters, helicopters, divers…).
    Class,
    /// An ally, a civilian or a guard (GameOver not 0, Alliance not NMI, or not hostile to the
    /// player when it appears: InitialAlliances gives the Player alliance a level of 0 or more).
    Side,
    /// A scene listed in EXCLUDED.
    Excluded,
    /// Named by a map script (MapInfo subclass, KillCounterTrigger, SoldierTouchAndKill,
    /// Patrouille, BossHealthBar), by object or by name.
    Named,
    /// Dead, being destroyed or generated.
    NotAlive,
    /// No AI controller to drive it (or one it must not get).
    NoAi,
}

impl Refusal {
    fn label(self) -> &'static str {
        match self {
            Refusal::Class => "class",
            Refusal::Side => "not hostile or GameOver",
            Refusal::Excluded => "excluded scene",
            Refusal::Named => "named by a script",
            Refusal::NotAlive => "not alive",
            Refusal::NoAi => "no AI",
        }
    }
}

/// What decides whether a placed soldier is copied.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SoldierFacts {
    pub(crate) exactly_base_soldier: bool,
    pub(crate) game_over: u8,
    pub(crate) alliance_nmi: bool,
    pub(crate) friendly_to_player: bool,
    pub(crate) named_by_script: bool,
    pub(crate) excluded: bool,
    pub(crate) alive: bool,
    pub(crate) ai: bool,
}

pub(crate) fn eligible(f: &SoldierFacts) -> Result<(), Refusal> {
    if !f.exactly_base_soldier {
        return Err(Refusal::Class);
    }
    if f.game_over != 0 || !f.alliance_nmi || f.friendly_to_player {
        return Err(Refusal::Side);
    }
    if f.named_by_script {
        return Err(Refusal::Named);
    }
    if f.excluded {
        return Err(Refusal::Excluded);
    }
    if !f.alive {
        return Err(Refusal::NotAlive);
    }
    if !f.ai {
        return Err(Refusal::NoAi);
    }
    Ok(())
}

/// Copies of the `k`-th eligible soldier (map order) for `rate` extra soldiers per soldier
/// (percent / 100 × extra players): 0.5 copies the second, fourth…, 1 copies each one once.
pub(crate) fn copies_for(k: usize, rate: f64) -> usize {
    let at = |n: usize| (n as f64 * rate + 1e-9).floor() as usize;
    at(k + 1) - at(k)
}

/// Extra soldiers for a span of `n` soldiers starting at rank `from` (all eligible soldiers of
/// the map counted in order), `rate` extra per soldier: the sum of copies_for over the span.
pub(crate) fn copies_for_span(from: usize, n: usize, rate: f64) -> usize {
    (from..from + n).map(|k| copies_for(k, rate)).sum()
}

/// Offsets (x, y) tried for the `copy`-th copy made on the map, ring by ring, nearest first.
pub(crate) fn ring_offsets(copy: usize) -> Vec<[f32; 2]> {
    let start = copy as f32 * GOLDEN_ANGLE;
    let mut out = Vec::new();
    for (distance, count) in RINGS {
        for i in 0..count {
            let angle = start + i as f32 * std::f32::consts::TAU / count as f32;
            out.push([distance * angle.dcos(), distance * angle.dsin()]);
        }
    }
    out
}

/// What the traces found at a place (see coop_enemies_ring_place).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PlaceProbe {
    /// Floor under the place: its height, the up part of its normal, world geometry (not a
    /// decoration, a vehicle, a body or a pawn). None: nothing within reach.
    pub(crate) floor: Option<(f32, f32, bool)>,
    /// The original's floor height.
    pub(crate) original_floor: f32,
    /// Straight way from the original at waist and head height (thin rays, level and blocking
    /// actors) and for a body (box), clear.
    pub(crate) way_clear: bool,
    /// Room above the floor for the soldier's height.
    pub(crate) headroom: bool,
    /// No pawn stands there.
    pub(crate) no_pawn: bool,
}

/// Why a place is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlaceRefusal {
    NoFloor,
    NotWorldFloor,
    Steep,
    OtherLevel,
    Blocked,
    NoHeadroom,
    Pawn,
}

pub(crate) fn accept_place(p: &PlaceProbe) -> Result<(), PlaceRefusal> {
    let Some((z, up, world)) = p.floor else { return Err(PlaceRefusal::NoFloor) };
    if !world {
        return Err(PlaceRefusal::NotWorldFloor);
    }
    if up < 0.7 {
        return Err(PlaceRefusal::Steep);
    }
    if (z - p.original_floor).abs() > MAX_STEP {
        return Err(PlaceRefusal::OtherLevel);
    }
    if !p.way_clear {
        return Err(PlaceRefusal::Blocked);
    }
    if !p.headroom {
        return Err(PlaceRefusal::NoHeadroom);
    }
    if !p.no_pawn {
        return Err(PlaceRefusal::Pawn);
    }
    Ok(())
}

/// A generator count scaled by `ratio` (1 + rate); -1 (unlimited) stays.
pub(crate) fn scale_count(n: i32, ratio: f64) -> i32 {
    if n < 0 {
        return n;
    }
    (n as f64 * ratio - 1e-9).ceil().max(n as f64) as i32
}

/// Group of the `k`-th soldier of a generator scaled by `ratio`: with 2 the pairs (0, 1),
/// (2, 3)…; with 1.5 (0, 1), (2), (3, 4), (5)…
pub(crate) fn generated_group(k: usize, ratio: f64) -> usize {
    (k as f64 / ratio + 1e-9).floor() as usize
}

impl Vm {
    /// Extra enemies in co-op: percent more per extra player (0 none, 50, 100), for the next
    /// maps. The co-op page sets it; the solo game never reads it.
    pub fn set_coop_extra_enemies(&mut self, percent: u32) {
        self.coop.enemies.percent = percent.min(300);
    }

    /// The setting above.
    pub fn coop_extra_enemies(&self) -> u32 {
        self.coop.enemies.percent
    }

    /// What the start of the current map did (co-op with extra enemies only).
    pub fn coop_enemies_report(&self) -> &Report {
        &self.coop.enemies.report
    }

    fn enemies_alive(&self, o: ObjId) -> bool {
        self.objects.get(o).is_some_and(|x| x.is_some()) && !self.prop(o, "bDeleteMe").as_bool()
    }

    /// A member still in the fight: not destroyed, not dead.
    fn enemies_fighting(&self, o: ObjId) -> bool {
        self.enemies_alive(o) && !self.prop(o, "bIsDead").as_bool() && self.prop(o, "Health").as_int() > 0
    }

    /// Start of the players' logins on a map: nothing from the previous map.
    pub(crate) fn coop_enemies_begin(&mut self) {
        let (percent, loud_spawns) = (self.coop.enemies.percent, self.coop.enemies.loud_spawns);
        self.coop.enemies = CoopEnemies { percent, loud_spawns, ..Default::default() };
    }

    /// After the players' logins of a co-op map, before its first frame: the copies and the
    /// generators' new numbers.
    pub(crate) fn coop_enemies_start(&mut self) {
        if !self.coop.on || self.coop.enemies.percent == 0 {
            return;
        }
        // Not on the character screen's backdrop map (character_screen.rs): nobody fights there.
        if crate::improved::gui::character_screen_open(self) {
            return;
        }
        let extra_players = self.coop.players.len().max(2) - 1;
        let rate = self.coop.enemies.percent as f64 / 100.0 * extra_players as f64;
        let mut report = Report::default();
        let named = self.coop_enemies_named();
        self.coop.enemies.donors = self.coop_enemies_donors(&named);
        self.coop_enemies_generators(1.0 + rate, &named, &mut report);
        self.coop_enemies_copies(rate, &named, &mut report);
        let line = format!(
            "Log: Co-op enemies: {} % per extra player; placed enemies {}, eligible {}, copies wanted {}, made {} ({} on a path node), no room {}, failed {:?}, refused {:?}; generators scaled {} (unlimited {}, left alone {}), soldiers {} -> {}",
            self.coop.enemies.percent, report.placed_enemies, report.eligible, report.wanted, report.made, report.on_node, report.no_room, report.failed, report.refused, report.generators, report.unlimited_generators, report.generators_refused, report.generated_before, report.generated_after
        );
        self.log.push(line);
        self.coop.enemies.report = report;
    }

    /// A generator may be scaled: an enemy generator not named by a script nor excluded.
    fn enemies_generator_refusal(&self, a: ObjId, named: &(HashSet<ObjId>, HashSet<String>)) -> Option<Refusal> {
        let own = self.objects[a].as_ref().map(|o| o.name.to_ascii_lowercase()).unwrap_or_default();
        let names = [own, self.prop(a, "Tag").name_ref().to_ascii_lowercase(), self.prop(a, "Bases_Tag").name_ref().to_ascii_lowercase()];
        if self.prop(a, "GameOver").as_byte() != 0 || !self.prop(a, "Alliance").name_ref().eq_ignore_ascii_case("NMI") || self.enemies_friendly(a) {
            Some(Refusal::Side)
        } else if named.0.contains(&a) || (!names[2].is_empty() && names[2] != "none" && named.1.contains(&names[2])) {
            // A script starting a generator by its Tag is how waves begin: only the Tag it gives
            // its soldiers (Bases_Tag) marks them as particular.
            Some(Refusal::Named)
        } else if self.enemies_excluded(&names) {
            Some(Refusal::Excluded)
        } else {
            None
        }
    }

    /// Look of a placed soldier, or of the soldiers a generator makes (gennmi gives them its
    /// Mesh if it has one, its first skin, its CodeMesh; their voice is drawn among
    /// TimbresPossibles).
    fn enemies_donor(&self, a: ObjId, generator: bool) -> Donor {
        let mut mesh = self.prop(a, "Mesh");
        if generator && matches!(mesh, Value::Object(None) | Value::Void) {
            if let Some(base) = self.class("BaseSoldier") {
                mesh = self.prop(self.classes[base].cdo, "Mesh");
            }
        }
        let skins = match self.prop(a, "Skins") {
            Value::Array(v) if generator => v.into_iter().take(1).filter(|s| !matches!(s, Value::Object(None) | Value::Void)).collect(),
            Value::Array(v) => v,
            _ => Vec::new(),
        };
        let casque = self.class("Casque");
        let helmet = match self.prop(a, "InitialInventory") {
            Value::Array(items) => items.iter().any(|i| matches!(i, Value::Struct(_, f) if f.first().and_then(Value::as_class).is_some_and(|c| casque.is_some_and(|h| self.is_child_of(c, h))))),
            _ => false,
        };
        let role = match self.prop(a, "SpecificAnimations") {
            Value::Array(v) => v.iter().any(|x| !x.to_string().contains("MigA")),
            _ => false,
        };
        Donor {
            look: Look { mesh, skins, code_mesh: self.prop(a, "CodeMesh").as_int(), timbre: (!generator).then(|| self.prop(a, "NumeroTimbre").as_byte()) },
            timbres: if generator { self.prop(a, "TimbresPossibles").name_ref().to_string() } else { String::new() },
            at: self.prop(a, "Location").as_vector(),
            network: self.prop(a, "NumReseauPropre").as_int(),
            alliance: self.prop(a, "Alliance").name_ref().to_string(),
            helmet,
            role,
        }
    }

    /// Looks of the eligible enemies of this map (placed soldiers and generators).
    fn coop_enemies_donors(&self, named: &(HashSet<ObjId>, HashSet<String>)) -> Vec<Donor> {
        let Some(base) = self.class("BaseSoldier") else { return Vec::new() };
        let mut out = Vec::new();
        for &a in &self.actors {
            if !self.enemies_alive(a) {
                continue;
            }
            if self.is_a(a, "gennmi") {
                if self.enemies_generator_refusal(a, named).is_none() {
                    out.push(self.enemies_donor(a, true));
                }
            } else if self.is_a(a, "BaseSoldier") && eligible(&self.coop_enemies_facts(a, base, named)).is_ok() {
                out.push(self.enemies_donor(a, false));
            }
        }
        out
    }

    /// The look of the `extra`-th extra of `original` (a placed soldier or a generator), with the
    /// looks its other extras got; None: it keeps its original's look.
    pub(crate) fn enemies_extra_look(&self, original: ObjId, generator: bool, extra: usize, taken: &[String]) -> Option<Look> {
        let own = self.enemies_donor(original, generator);
        let name = self.objects[original].as_ref().map(|o| o.name.clone()).unwrap_or_default();
        let mut rng = LookRng::new(&self.url.map, &name, extra);
        let i = choose_look(&own, &self.coop.enemies.donors, taken, &mut rng)?;
        let donor = &self.coop.enemies.donors[i];
        let mut look = donor.look.clone();
        if look.timbre.is_none() && !generator {
            look.timbre = timbre_of(&donor.timbres, &mut rng);
        }
        Some(look)
    }

    /// Mods build (network guests): the skins an extra was given cross the network like a mod's.
    #[allow(unused_variables)]
    fn enemies_note_skins(&mut self, pawn: ObjId, skins: &[Value]) {
        #[cfg(feature = "mods")]
        if !skins.is_empty() {
            let Some(class) = self.objects.get(pawn).and_then(|o| o.as_ref()).map(|o| self.class_path(o.class)) else { return };
            let names = skins
                .iter()
                .map(|v| match v {
                    Value::Asset(s) if !s.is_empty() => s.clone(),
                    Value::Object(Some(o)) => self.path_name(*o),
                    _ => "None".into(),
                })
                .collect();
            self.mods.net_skins.insert(pawn, crate::mod_net_skins::NetSkins { class, skins: names });
        }
    }

    /// Name and Tag of an actor, for the decisions list.
    fn enemies_label(&self, a: ObjId) -> String {
        let own = self.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default();
        format!("{own} (Tag {})", self.prop(a, "Tag").name_ref())
    }

    /// The Player alliance has a level of 0 or more in InitialAlliances: not hostile to the
    /// player when it appears.
    fn enemies_friendly(&self, a: ObjId) -> bool {
        let Value::Array(items) = self.prop(a, "InitialAlliances") else { return false };
        items.iter().any(|i| match i {
            Value::Struct(_, f) => f.first().is_some_and(|n| n.name_ref().eq_ignore_ascii_case("Player")) && f.get(1).is_some_and(|l| l.as_float() >= 0.0),
            _ => false,
        })
    }

    /// One of `names` (lower case) is in this map's EXCLUDED list.
    fn enemies_excluded(&self, names: &[String]) -> bool {
        let map = self.url.map.to_ascii_lowercase();
        EXCLUDED.iter().any(|(m, n)| map == *m && names.iter().any(|x| x == n))
    }

    /// gennmi: NbNMIMax, NbNMIActifs and NbNMIGen × `ratio` (enemy generators only).
    fn coop_enemies_generators(&mut self, ratio: f64, named: &(HashSet<ObjId>, HashSet<String>), report: &mut Report) {
        let mut made_so_far = 0usize;
        for a in self.actors.clone() {
            if !self.enemies_alive(a) || !self.is_a(a, "gennmi") {
                continue;
            }
            let label = format!("generator {}", self.enemies_label(a));
            if let Some(why) = self.enemies_generator_refusal(a, named) {
                report.generators_refused += 1;
                report.decisions.push(format!("{label}: {}", why.label()));
                continue;
            }
            let max = self.prop(a, "NbNMIMax").as_int();
            if max < 0 {
                report.unlimited_generators += 1;
                report.decisions.push(format!("{label}: unlimited, left alone"));
                continue;
            }
            report.generators += 1;
            report.generated_before += max as i64;
            // Extra soldiers shared out over the map's generators in order, as for the placed
            // soldiers (50 %: one generator of one soldier out of two gets a second one).
            let extra = copies_for_span(made_so_far, max as usize, ratio - 1.0);
            made_so_far += max as usize;
            if max == 0 || extra == 0 {
                report.generated_after += max as i64;
                report.decisions.push(format!("{label}: kept, {max} -> {max} (its turn comes later)"));
                continue;
            }
            let after = max + extra as i32;
            let ratio = after as f64 / max as f64;
            self.set_prop(a, "NbNMIMax", Value::Int(after));
            for name in ["NbNMIActifs", "NbNMIGen"] {
                let n = self.prop(a, name).as_int();
                self.set_prop(a, name, Value::Int(scale_count(n, ratio)));
            }
            report.generated_after += after as i64;
            report.decisions.push(format!("{label}: kept, {max} -> {after}"));
            self.coop.enemies.generators.insert(a, Generator { ratio, spawned: 0 });
        }
    }
    /// Objects and names that map scripts keep to reach particular soldiers.
    fn coop_enemies_named(&self) -> (HashSet<ObjId>, HashSet<String>) {
        let (mut objects, mut names) = (HashSet::new(), HashSet::new());
        fn collect(v: &Value, objects: &mut HashSet<ObjId>, names: &mut HashSet<String>) {
            match v {
                Value::Object(Some(o)) => {
                    objects.insert(*o);
                }
                Value::Name(n) => {
                    let n = n.as_str();
                    if !n.is_empty() && !n.eq_ignore_ascii_case("None") {
                        names.insert(n.to_ascii_lowercase());
                    }
                }
                Value::Array(items) => items.iter().for_each(|i| collect(i, objects, names)),
                Value::Struct(_, fields) => fields.iter().for_each(|i| collect(i, objects, names)),
                _ => {}
            }
        }
        let map_info = self.class("MapInfo");
        let referrers: Vec<_> = SCRIPT_REFERRERS.iter().filter_map(|n| self.class(n)).collect();
        for &a in &self.actors {
            let Some(o) = self.objects.get(a).and_then(|o| o.as_ref()) else { continue };
            // Properties declared from this class down (not the Actor ones: its own Tag, Event…).
            let from = if map_info.is_some_and(|m| o.class != m && self.is_child_of(o.class, m)) {
                map_info
            } else {
                referrers.iter().copied().find(|&r| self.is_child_of(o.class, r))
            };
            let Some(from) = from else { continue };
            let start = if Some(from) == map_info { self.classes[from].props.len() } else { self.classes[from].super_class.map_or(0, |s| self.classes[s].props.len()) };
            for v in o.props.iter().skip(start) {
                collect(v, &mut objects, &mut names);
            }
        }
        (objects, names)
    }

    fn coop_enemies_facts(&self, a: ObjId, base: crate::value::ClassId, named: &(HashSet<ObjId>, HashSet<String>)) -> SoldierFacts {
        let class = self.objects[a].as_ref().map(|o| o.class);
        let tag = self.prop(a, "Tag").name_ref().to_ascii_lowercase();
        let own = self.objects[a].as_ref().map(|o| o.name.to_ascii_lowercase()).unwrap_or_default();
        let controller = self.prop(a, "Controller").as_object().filter(|&c| self.enemies_alive(c));
        let ai = match controller {
            Some(c) => self.is_a(c, "IAController") && !self.is_a(c, "PlayerController"),
            None => false,
        };
        SoldierFacts {
            exactly_base_soldier: class == Some(base),
            game_over: self.prop(a, "GameOver").as_byte(),
            alliance_nmi: self.prop(a, "Alliance").name_ref().eq_ignore_ascii_case("NMI"),
            friendly_to_player: self.enemies_friendly(a),
            named_by_script: named.0.contains(&a) || named.1.contains(&tag) || named.1.contains(&own),
            excluded: self.enemies_excluded(&[tag.clone(), own.clone()]),
            alive: self.enemies_fighting(a) && !self.prop(a, "bBasesGenere").as_bool(),
            ai: ai && !self.prop(a, "bDontPossess").as_bool(),
        }
    }

    /// Copies of the placed soldiers.
    fn coop_enemies_copies(&mut self, rate: f64, named: &(HashSet<ObjId>, HashSet<String>), report: &mut Report) {
        let Some(base) = self.class("BaseSoldier") else { return };
        let soldiers: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.enemies_alive(a) && self.is_a(a, "BaseSoldier")).collect();
        let mut refused: Vec<(String, usize)> = Vec::new();
        let mut chosen = Vec::new();
        for &a in &soldiers {
            let facts = self.coop_enemies_facts(a, base, named);
            if facts.exactly_base_soldier && facts.game_over == 0 && facts.alliance_nmi {
                report.placed_enemies += 1;
            }
            let label = format!("soldier {} [{}]", self.enemies_label(a), self.objects[a].as_ref().map(|o| self.classes[o.class].name.clone()).unwrap_or_default());
            match eligible(&facts) {
                Ok(()) => {
                    report.decisions.push(format!("{label}: kept"));
                    chosen.push(a)
                }
                Err(why) => match {
                    report.decisions.push(format!("{label}: {}", why.label()));
                    refused.iter_mut().find(|(l, _)| l == why.label())
                } {
                    Some(r) => r.1 += 1,
                    None => refused.push((why.label().to_string(), 1)),
                },
            }
        }
        report.eligible = chosen.len();
        report.refused = refused;
        // Pawns already standing (a copy must not be put inside one), path nodes.
        let mut pawns: Vec<([f32; 3], f32, f32)> = self
            .actors
            .iter()
            .copied()
            .filter(|&a| self.enemies_alive(a) && self.is_a(a, "Pawn"))
            .map(|a| (self.prop(a, "Location").as_vector(), self.prop(a, "CollisionRadius").as_float(), self.prop(a, "CollisionHeight").as_float()))
            .collect();
        let nodes: Vec<[f32; 3]> = self.actors.iter().copied().filter(|&a| self.enemies_alive(a) && self.is_a(a, "PathNode")).map(|a| self.prop(a, "Location").as_vector()).collect();
        for (k, &original) in chosen.iter().enumerate() {
            let n = copies_for(k, rate);
            if n == 0 {
                continue;
            }
            report.wanted += n;
            let mut group = Group { members: vec![original], dead: vec![false], placed: true, places: vec![None] };
            let mut taken: Vec<String> = Vec::new();
            for extra in 0..n {
                // Its look and voice, from another enemy of the map (see choose_look).
                let look = self.enemies_extra_look(original, false, extra, &taken);
                if let Some(l) = &look {
                    taken.push(l.key());
                }
                self.coop.enemies.template_look = look.clone();
                let made = self.coop_enemies_copy(original, &mut pawns, &nodes);
                self.coop.enemies.template_look = None;
                if let Ok((copy, _, _)) = made {
                    let before = self.enemies_donor(original, false).look.key();
                    let line = format!("{} of {}: {before} -> {}", self.enemies_label(copy), self.enemies_label(original), look.as_ref().map_or_else(|| "same".to_string(), |l| l.key()));
                    report.looks.push(line);
                    if let Some(l) = look {
                        self.enemies_note_skins(copy, &l.skins);
                    }
                }
                match made {
                    Ok((copy, place, on_node)) => {
                        report.made += 1;
                        report.on_node += usize::from(on_node);
                        group.members.push(copy);
                        group.dead.push(false);
                        group.places.push(Some(place));
                    }
                    Err("no room") => report.no_room += 1,
                    Err(why) => match report.failed.iter_mut().find(|(l, _)| l == why) {
                        Some(r) => r.1 += 1,
                        None => report.failed.push((why.to_string(), 1)),
                    },
                }
            }
            if group.members.len() > 1 {
                let id = self.coop.enemies.groups.len();
                for &m in &group.members {
                    self.coop.enemies.member.insert(m, id);
                }
                self.coop.enemies.groups.push(group);
            }
        }
    }

    /// A free place for the `copy`-th copy of the map, a copy of `original`: on rings around it
    /// (coop_enemies_ring_place), else a close path node it can see. (place, on a path node)
    fn coop_enemies_place(&mut self, original: ObjId, copy: usize, pawns: &[([f32; 3], f32, f32)], nodes: &[[f32; 3]]) -> Option<([f32; 3], bool)> {
        let (radius, height) = (self.prop(original, "CollisionRadius").as_float(), self.prop(original, "CollisionHeight").as_float());
        let free = |p: [f32; 3]| {
            pawns.iter().all(|&(q, r, h)| {
                let flat = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
                flat >= radius + r + PAWN_GAP || (p[2] - q[2]).abs() >= height + h
            })
        };
        let origin = self.prop(original, "Location").as_vector();
        let trace = self.find_function(original, "Trace", false)?;
        let original_floor = self.enemies_floor(original, trace, origin, origin[2] + 1.0, origin[2] - height - MAX_STEP - 200.0).map_or(origin[2] - height, |(z, _, _)| z);
        let mut refusals: Vec<(PlaceRefusal, usize)> = Vec::new();
        for [dx, dy] in ring_offsets(copy) {
            match self.coop_enemies_ring_place(original, trace, origin, original_floor, [origin[0] + dx, origin[1] + dy], &free) {
                Ok(place) => return Some((place, false)),
                Err(why) => match refusals.iter_mut().find(|r| r.0 == why) {
                    Some(r) => r.1 += 1,
                    None => refusals.push((why, 1)),
                },
            }
        }
        let _ = &refusals;
        let mut close: Vec<([f32; 3], f32)> = nodes
            .iter()
            .map(|&n| (n, ((n[0] - origin[0]).powi(2) + (n[1] - origin[1]).powi(2) + (n[2] - origin[2]).powi(2)).sqrt()))
            .filter(|&(_, d)| d <= NODE_DISTANCE)
            .collect();
        close.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (node, _) in close {
            let v = |p: [f32; 3]| Value::vector(p[0], p[1], p[2]);
            if free(node) && self.call_named(original, "FastTrace", vec![v(node), v(origin)]).is_ok_and(|r| r.as_bool()) {
                return Some((node, true));
            }
        }
        None
    }

    /// Floor under (x, y) of `at` between heights `from` and `to`, by a thin ray (a static mesh
    /// floor blocks thin rays, not always boxes): (height, up part of the normal, world
    /// geometry: level, static mesh, terrain, not a decoration, a vehicle, a body or a pawn).
    fn enemies_floor(&mut self, by: ObjId, trace: FuncId, at: [f32; 3], from: f32, to: f32) -> Option<(f32, f32, bool)> {
        let v = |p: [f32; 3]| Some(Value::vector(p[0], p[1], p[2]));
        let args = vec![v([0.0; 3]), v([0.0; 3]), v([at[0], at[1], to]), v([at[0], at[1], from]), Some(Value::Bool(true)), v([0.0; 3])];
        let (hit, outs) = self.call(trace, by, args).ok()?;
        let ground = hit.as_object()?;
        let z = outs.first()?.as_ref()?.as_vector()[2];
        let up = outs.get(1)?.as_ref()?.as_vector()[2];
        let world = (self.is_a(ground, "LevelInfo") || self.prop(ground, "bWorldGeometry").as_bool() || self.is_a(ground, "StaticMeshActor") || self.is_a(ground, "TerrainInfo"))
            && !self.is_a(ground, "Decoration")
            && !self.is_a(ground, "Pawn");
        Some((z, up, world))
    }

    /// Nothing between `from` and `to` (thin ray with the blocking actors, or box `extent`).
    fn enemies_clear(&mut self, by: ObjId, trace: FuncId, from: [f32; 3], to: [f32; 3], extent: [f32; 3]) -> bool {
        let v = |p: [f32; 3]| Some(Value::vector(p[0], p[1], p[2]));
        let args = vec![v([0.0; 3]), v([0.0; 3]), v(to), v(from), Some(Value::Bool(true)), v(extent)];
        let hit = self.call(trace, by, args).ok().and_then(|(hit, _)| hit.as_object());
        hit.is_none()
    }

    /// The place at (x, y) for a copy of `original` if it passes accept_place: a pawn's centre.
    fn coop_enemies_ring_place(&mut self, original: ObjId, trace: FuncId, origin: [f32; 3], original_floor: f32, xy: [f32; 2], free: &dyn Fn([f32; 3]) -> bool) -> Result<[f32; 3], PlaceRefusal> {
        let (radius, height) = (self.prop(original, "CollisionRadius").as_float(), self.prop(original, "CollisionHeight").as_float());
        let floor = self.enemies_floor(original, trace, [xy[0], xy[1], 0.0], origin[2] + GROUND_ABOVE, original_floor - MAX_STEP - 10.0);
        let mut probe = PlaceProbe { floor, original_floor, ..Default::default() };
        // Checks cost traces: stop at the first refusal.
        accept_place(&PlaceProbe { way_clear: true, headroom: true, no_pawn: true, ..probe })?;
        let z = floor.ok_or(PlaceRefusal::NoFloor)?.0;
        let place = [xy[0], xy[1], z + height + 2.0];
        let waist = |f: f32| [xy[0], xy[1], f];
        // Way: a thin ray at the waist from the original (static meshes, blocking actors; a box
        // from the original would start inside what it leans on). Room: a body-sized box,
        // from a little above the floor (steps and small bumps allowed) to below the head,
        // lowered onto the place: it must meet nothing.
        let body = [radius, radius, (height - 30.0).max(10.0)];
        let at = [place[0], place[1], place[2] + 10.0];
        probe.way_clear = self.enemies_clear(original, trace, [origin[0], origin[1], original_floor + height], waist(z + height), [0.0; 3])
            && self.enemies_clear(original, trace, [at[0], at[1], at[2] + 30.0], at, body);
        probe.headroom = probe.way_clear && self.enemies_clear(original, trace, [xy[0], xy[1], z + 4.0], [xy[0], xy[1], z + 2.0 * height + 4.0], [0.0; 3]);
        probe.no_pawn = free(place);
        accept_place(&probe).map(|()| place)
    }
    /// One copy of `original`: (copy, place, on a path node).
    fn coop_enemies_copy(&mut self, original: ObjId, pawns: &mut Vec<([f32; 3], f32, f32)>, nodes: &[[f32; 3]]) -> Result<(ObjId, [f32; 3], bool), &'static str> {
        let copy = self.coop.enemies.copies_tried;
        self.coop.enemies.copies_tried += 1;
        let (place, on_node) = self.coop_enemies_place(original, copy, pawns, nodes).ok_or("no room")?;
        let class = self.objects[original].as_ref().ok_or("gone")?.class;
        let tag = self.prop(original, "Tag");
        let rotation = self.prop(original, "Rotation");
        self.coop.enemies.template = Some(original);
        let spawned = self.spawn_actor(class, Value::Object(None), Some(tag), Value::vector(place[0], place[1], place[2]), rotation, Value::Object(None), false);
        self.coop.enemies.template = None;
        self.coop.enemies.quiet_spawn = None;
        let copy = match spawned {
            Ok(Some(c)) if self.enemies_alive(c) => c,
            Ok(_) => return Err("spawn refused"),
            Err(e) => {
                self.log.push(format!("Log: Co-op enemies: copy of {} failed: {e}", self.path_name(original)));
                return Err("spawn failed");
            }
        };
        // Its AI, as gennmi does for the soldiers it makes after the map's start; before its
        // collision is set, so that the volumes and triggers it touches see a pawn with its
        // controller (DetectionVolume.Touch reads Other.Controller).
        let controller_class = self.prop(copy, "ControllerClass").as_class();
        if self.prop(copy, "Controller").as_object().is_none() {
            if let Some(cc) = controller_class {
                let at = self.prop(copy, "Location");
                let rot = self.prop(copy, "Rotation");
                if let Ok(Some(c)) = self.spawn_actor(cc, Value::Object(None), None, at, rot, Value::Object(None), true) {
                    let _ = self.call_named(c, "Possess", vec![Value::Object(Some(copy))]);
                }
            }
        }
        if let Some(c) = self.prop(copy, "Controller").as_object().filter(|&c| self.enemies_alive(c)) {
            let focal = self.prop(self.prop(original, "Controller").as_object().unwrap_or(original), "FocalPoint");
            if !matches!(focal, Value::Void) {
                self.set_prop(c, "FocalPoint", focal);
            }
        } else {
            let _ = self.destroy_actor(copy);
            return Err("no AI");
        }
        // Collision as the original's (its natives keep the collision hash right).
        let size = ["CollisionRadius", "CollisionHeight"].map(|n| self.prop(original, n));
        let _ = self.call_named(copy, "SetCollisionSize", size.to_vec());
        let flags = ["bCollideActors", "bBlockActors", "bBlockPlayers"].map(|n| self.prop(original, n));
        let _ = self.call_named(copy, "SetCollision", flags.to_vec());
        // The game's list of soldiers (GameInfo.InitLists does it for the placed ones).
        if let Some(game) = self.level.and_then(|l| self.prop(l, "Game").as_object()) {
            if let Value::Array(mut list) = self.prop(game, "BaseSoldierList") {
                list.push(Value::Object(Some(copy)));
                self.set_prop(game, "BaseSoldierList", Value::Array(list));
            }
        }
        let (r, h) = (self.prop(copy, "CollisionRadius").as_float(), self.prop(copy, "CollisionHeight").as_float());
        let at = self.prop(copy, "Location").as_vector();
        pawns.push((at, r, h));
        Ok((copy, at, on_node))
    }

    /// In spawn_actor, before PreBeginPlay: a copy takes the editable properties of its
    /// original (as the map gave them), its weapons and ammunition but not its story items.
    pub(crate) fn coop_enemies_spawn_values(&mut self, a: ObjId) {
        let Some(original) = self.coop.enemies.template.take() else { return };
        if !self.coop.enemies.loud_spawns {
            self.coop.enemies.quiet_spawn = Some(a);
        }
        let (Some(class), Some(from)) = (self.objects[a].as_ref().map(|o| o.class), self.objects[original].as_ref().map(|o| o.class)) else { return };
        if class != from {
            return;
        }
        let props: Vec<(usize, String)> = self.classes[class]
            .props
            .iter()
            .enumerate()
            .filter(|(_, p)| p.flags & 1 != 0 && !COLLISION_PROPS.contains(&p.name.as_str()) && !NEVER_COPIED.contains(&p.name.as_str()))
            .map(|(i, p)| (i, p.name.clone()))
            .collect();
        let values: Vec<(usize, Value)> = props.iter().filter_map(|(i, _)| self.objects[original].as_ref().map(|o| (*i, o.props[*i].clone()))).collect();
        if let Some(o) = self.objects[a].as_mut() {
            for (i, v) in values {
                o.props[i] = v;
            }
        }
        // Weapons and ammunition only.
        let (weapon, ammo) = (self.class("Weapon"), self.class("Ammunition"));
        if let Value::Array(mut items) = self.prop(original, "InitialInventory") {
            for item in items.iter_mut() {
                let Value::Struct(_, fields) = item else { continue };
                let keep = fields.first().and_then(Value::as_class).is_some_and(|c| weapon.is_some_and(|w| self.is_child_of(c, w)) || ammo.is_some_and(|m| self.is_child_of(c, m)));
                if !keep && fields.len() >= 2 {
                    fields[0] = Value::Class(None);
                    fields[1] = Value::Int(0);
                }
            }
            self.set_prop(a, "InitialInventory", Value::Array(items));
        }
        // Another enemy's look and voice (its animation links stay its original's).
        if let Some(look) = self.coop.enemies.template_look.take() {
            self.set_prop(a, "Mesh", look.mesh);
            self.set_prop(a, "Skins", Value::Array(look.skins));
            self.set_prop(a, "CodeMesh", Value::Int(look.code_mesh));
            if let Some(t) = look.timbre {
                self.set_prop(a, "NumeroTimbre", Value::Byte(t));
            }
        }
    }

    /// Before a function call in co-op: deaths of grouped soldiers, soldiers made by a scaled
    /// generator.
    pub(crate) fn coop_enemies_before_call(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) {
        let name = self.functions[f].name.as_str();
        if name.eq_ignore_ascii_case("Died") {
            if let Some(&g) = self.coop.enemies.member.get(&this) {
                self.coop_enemies_died(g, this);
            }
        } else if name.eq_ignore_ascii_case("GeneratedPawn") && !self.coop.enemies.generators.is_empty() {
            let generator = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object);
            let pawn = args.get(1).and_then(|a| a.as_ref()).and_then(Value::as_object);
            if let (Some(generator), Some(pawn)) = (generator, pawn) {
                self.coop_enemies_generated(generator, pawn);
            }
        }
    }

    /// gennmi made `pawn`: it joins its group.
    fn coop_enemies_generated(&mut self, generator: ObjId, pawn: ObjId) {
        let Some(state) = self.coop.enemies.generators.get_mut(&generator) else { return };
        let k = state.spawned;
        state.spawned += 1;
        let number = generated_group(k, state.ratio);
        let id = match self.coop.enemies.generated_groups.get(&(generator, number)) {
            Some(&id) => id,
            None => {
                let id = self.coop.enemies.groups.len();
                self.coop.enemies.groups.push(Group::default());
                self.coop.enemies.generated_groups.insert((generator, number), id);
                id
            }
        };
        let group = &mut self.coop.enemies.groups[id];
        group.members.push(pawn);
        group.dead.push(false);
        group.places.push(None);
        let extra = group.members.len() > 1;
        self.coop.enemies.member.insert(pawn, id);
        // Look: gennmi dresses its soldier right after this call (Mesh, first skin, CodeMesh,
        // voice among TimbresPossibles by the game's own draw, unchanged): an extra is dressed
        // from another enemy's look lent to the generator for that moment, an original from the
        // generator's own.
        const DRESS: [&str; 3] = ["Mesh", "Skins", "CodeMesh"];
        let own = self.coop.enemies.generator_looks.entry(generator).or_insert_with(|| [Value::Void, Value::Void, Value::Void]).clone();
        let own = if matches!(own[0], Value::Void) {
            let saved = DRESS.map(|n| self.prop(generator, n));
            self.coop.enemies.generator_looks.insert(generator, saved.clone());
            saved
        } else {
            own
        };
        for (n, v) in DRESS.iter().zip(own) {
            self.set_prop(generator, n, v);
        }
        let before = self.enemies_donor(generator, true).look.key();
        let taken = self.coop.enemies.generator_taken.get(&generator).cloned().unwrap_or_default();
        let look = if extra { self.enemies_extra_look(generator, true, k, &taken) } else { None };
        if let Some(l) = &look {
            self.set_prop(generator, "Mesh", l.mesh.clone());
            self.set_prop(generator, "Skins", Value::Array(l.skins.clone()));
            self.set_prop(generator, "CodeMesh", Value::Int(l.code_mesh));
            self.coop.enemies.generator_taken.entry(generator).or_default().push(l.key());
            self.enemies_note_skins(pawn, &l.skins);
        }
        if extra {
            let line = format!("{} of {}: {before} -> {}", self.enemies_label(pawn), self.enemies_label(generator), look.as_ref().map_or_else(|| "same".to_string(), |l| l.key()));
            self.coop.enemies.report.looks.push(line);
        }
    }

    /// Group member `this` is dying (its Died runs next): while another member fights on, its
    /// death Event moves to that one.
    fn coop_enemies_died(&mut self, g: usize, this: ObjId) {
        let Some(rank) = self.coop.enemies.groups[g].members.iter().position(|&m| m == this) else { return };
        if self.coop.enemies.groups[g].dead[rank] {
            return;
        }
        self.coop.enemies.groups[g].dead[rank] = true;
        let named = |v: &Value| !v.name_ref().is_empty() && !v.name_ref().eq_ignore_ascii_case("None");
        let event = self.prop(this, "Event");
        let survivor = self.coop.enemies.groups[g].members.clone().into_iter().enumerate().find(|&(i, m)| i != rank && !self.coop.enemies.groups[g].dead[i] && self.enemies_fighting(m)).map(|(_, m)| m);
        let moved = match survivor {
            Some(s) if named(&event) => {
                self.set_prop(this, "Event", Value::Name(NameStruct::from("None".to_string())));
                if !named(&self.prop(s, "Event")) {
                    self.set_prop(s, "Event", event.clone());
                }
                Some(s)
            }
            _ => None,
        };
        let _ = moved;
    }

    /// Once per frame: copies of a placed soldier the story removed (alive) go with it.
    pub(crate) fn coop_enemies_frame(&mut self) {
        if self.coop.enemies.groups.is_empty() {
            return;
        }
        for g in 0..self.coop.enemies.groups.len() {
            let group = &self.coop.enemies.groups[g];
            if !group.placed || group.dead[0] || self.enemies_alive(group.members[0]) {
                continue;
            }
            let copies: Vec<ObjId> = group.members[1..].to_vec();
            self.coop.enemies.groups[g].dead[0] = true;
            for c in copies {
                if self.enemies_fighting(c) {
                    let _ = self.destroy_actor(c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ordinary() -> SoldierFacts {
        SoldierFacts { exactly_base_soldier: true, game_over: 0, alliance_nmi: true, friendly_to_player: false, named_by_script: false, excluded: false, alive: true, ai: true }
    }

    #[test]
    fn eligibility() {
        assert_eq!(eligible(&ordinary()), Ok(()));
        // Each refusal on its own (negative controls: one fact changed each time).
        assert_eq!(eligible(&SoldierFacts { exactly_base_soldier: false, ..ordinary() }), Err(Refusal::Class));
        assert_eq!(eligible(&SoldierFacts { game_over: 1, ..ordinary() }), Err(Refusal::Side));
        assert_eq!(eligible(&SoldierFacts { alliance_nmi: false, ..ordinary() }), Err(Refusal::Side));
        assert_eq!(eligible(&SoldierFacts { friendly_to_player: true, ..ordinary() }), Err(Refusal::Side));
        assert_eq!(eligible(&SoldierFacts { named_by_script: true, ..ordinary() }), Err(Refusal::Named));
        assert_eq!(eligible(&SoldierFacts { excluded: true, ..ordinary() }), Err(Refusal::Excluded));
        assert_eq!(eligible(&SoldierFacts { alive: false, ..ordinary() }), Err(Refusal::NotAlive));
        assert_eq!(eligible(&SoldierFacts { ai: false, ..ordinary() }), Err(Refusal::NoAi));
    }

    #[test]
    fn half_copies_one_soldier_out_of_two() {
        let half: Vec<usize> = (0..8).map(|k| copies_for(k, 0.5)).collect();
        assert_eq!(half, [0, 1, 0, 1, 0, 1, 0, 1]);
        assert_eq!((0..101).map(|k| copies_for(k, 0.5)).sum::<usize>(), 50);
        // Stable: the same answer every time, and the full setting copies every soldier once.
        assert_eq!(half, (0..8).map(|k| copies_for(k, 0.5)).collect::<Vec<_>>());
        assert!((0..50).all(|k| copies_for(k, 1.0) == 1));
        assert!((0..50).all(|k| copies_for(k, 2.0) == 2));
        assert!((0..50).all(|k| copies_for(k, 0.0) == 0));
        // Negative control: a different rule (copy the first of each pair) would not match.
        assert_ne!(half, (0..8).map(|k| usize::from(k % 2 == 0)).collect::<Vec<_>>());
    }

    #[test]
    fn generator_scaling() {
        assert_eq!(scale_count(-1, 2.0), -1);
        assert_eq!(scale_count(0, 2.0), 0);
        assert_eq!(scale_count(10, 2.0), 20);
        assert_eq!(scale_count(3, 1.5), 5);
        assert_eq!(scale_count(1, 1.5), 2);
        assert_eq!(scale_count(2, 1.5), 3);
        assert_eq!(scale_count(7, 1.0), 7);
        // Every original count comes back as as many groups.
        for ratio in [1.0, 1.5, 2.0, 3.0] {
            for n in 0..40 {
                let m = scale_count(n, ratio) as usize;
                let groups = (0..m).map(|k| generated_group(k, ratio)).collect::<HashSet<_>>().len();
                assert_eq!(groups, n as usize, "n {n} ratio {ratio}");
                // At most ceil(ratio) soldiers per group.
                for g in 0..n as usize {
                    let size = (0..m).filter(|&k| generated_group(k, ratio) == g).count();
                    assert!(size >= 1 && size <= ratio.ceil() as usize);
                }
            }
        }
        assert_eq!((0..6).map(|k| generated_group(k, 1.5)).collect::<Vec<_>>(), [0, 0, 1, 2, 2, 3]);
        assert_eq!((0..4).map(|k| generated_group(k, 2.0)).collect::<Vec<_>>(), [0, 0, 1, 1]);
        // 50 % over generators of one soldier each: every second one gets a second soldier.
        let spans: Vec<usize> = (0..6).map(|g| copies_for_span(g, 1, 0.5)).collect();
        assert_eq!(spans, [0, 1, 0, 1, 0, 1]);
        assert_eq!(copies_for_span(0, 10, 0.5), 5);
        assert_eq!(copies_for_span(3, 4, 1.0), 4);
    }
    #[test]
    fn rings_nearest_first_and_spread() {
        let total: usize = RINGS.iter().map(|r| r.1).sum();
        for copy in 0..6 {
            let rings = ring_offsets(copy);
            assert_eq!(rings.len(), total);
            let distances: Vec<f32> = rings.iter().map(|o| o[0].dhypot(o[1])).collect();
            assert!(distances.windows(2).all(|w| w[0] <= w[1] + 0.01), "nearest ring first");
            // Each ring is a full turn: its places average out at the original.
            let mut i = 0;
            for (d, n) in RINGS {
                let ring = &rings[i..i + n];
                let (sx, sy) = ring.iter().fold((0.0, 0.0), |a, o| (a.0 + o[0], a.1 + o[1]));
                assert!(sx.abs() < 1.0 && sy.abs() < 1.0 && ring.iter().all(|o| (o[0].dhypot(o[1]) - d).abs() < 0.01));
                i += n;
            }
        }
        // Copies start at different angles (negative control: the same copy starts the same).
        let first = |c: usize| ring_offsets(c)[0];
        assert_eq!(first(3), first(3));
        for (a, b) in [(0, 1), (1, 2), (0, 2), (2, 3)] {
            let (p, q) = (first(a), first(b));
            assert!((p[0] - q[0]).dhypot(p[1] - q[1]) > 10.0, "copies {a} and {b}");
        }
    }

    #[test]
    fn place_acceptance() {
        let good = PlaceProbe { floor: Some((100.0, 1.0, true)), original_floor: 100.0, way_clear: true, headroom: true, no_pawn: true };
        assert_eq!(accept_place(&good), Ok(()));
        assert_eq!(accept_place(&PlaceProbe { floor: Some((100.0 + MAX_STEP, 1.0, true)), ..good }), Ok(()));
        assert_eq!(accept_place(&PlaceProbe { floor: Some((100.0 - MAX_STEP, 0.8, true)), ..good }), Ok(()));
        // Each rule on its own (negative controls).
        assert_eq!(accept_place(&PlaceProbe { floor: None, ..good }), Err(PlaceRefusal::NoFloor));
        assert_eq!(accept_place(&PlaceProbe { floor: Some((100.0, 1.0, false)), ..good }), Err(PlaceRefusal::NotWorldFloor));
        assert_eq!(accept_place(&PlaceProbe { floor: Some((100.0, 0.6, true)), ..good }), Err(PlaceRefusal::Steep));
        assert_eq!(accept_place(&PlaceProbe { floor: Some((100.0 + MAX_STEP + 1.0, 1.0, true)), ..good }), Err(PlaceRefusal::OtherLevel));
        assert_eq!(accept_place(&PlaceProbe { floor: Some((100.0 - MAX_STEP - 1.0, 1.0, true)), ..good }), Err(PlaceRefusal::OtherLevel));
        assert_eq!(accept_place(&PlaceProbe { way_clear: false, ..good }), Err(PlaceRefusal::Blocked));
        assert_eq!(accept_place(&PlaceProbe { headroom: false, ..good }), Err(PlaceRefusal::NoHeadroom));
        assert_eq!(accept_place(&PlaceProbe { no_pawn: false, ..good }), Err(PlaceRefusal::Pawn));
    }
    fn donor(key: &str, at: [f32; 3], network: i32, alliance: &str, helmet: bool, role: bool) -> Donor {
        Donor { look: Look { mesh: Value::Asset(key.into()), skins: Vec::new(), code_mesh: 9, timbre: Some(0) }, timbres: String::new(), at, network, alliance: alliance.into(), helmet, role }
    }

    #[test]
    fn look_draw_is_seeded_and_stable() {
        let seq = |m: &str, w: &str, e: usize| {
            let mut r = LookRng::new(m, w, e);
            (0..8).map(|_| r.next()).collect::<Vec<_>>()
        };
        assert_eq!(seq("Hual01a", "BaseSoldier3", 0), seq("hual01a", "basesoldier3", 0), "case does not matter");
        // Negative controls: another map, enemy or extra gives another draw.
        assert_ne!(seq("Hual01a", "BaseSoldier3", 0), seq("Hual01a", "BaseSoldier3", 1));
        assert_ne!(seq("Hual01a", "BaseSoldier3", 0), seq("Hual01a", "BaseSoldier4", 0));
        assert_ne!(seq("Hual01a", "BaseSoldier3", 0), seq("Plage01", "BaseSoldier3", 0));
        let original = donor("A", [0.0; 3], 0, "NMI", false, false);
        let donors: Vec<Donor> = ["B", "C", "D", "E"].iter().map(|k| donor(k, [100.0, 0.0, 0.0], 0, "NMI", false, false)).collect();
        let pick = |e: usize| choose_look(&original, &donors, &[], &mut LookRng::new("M", "S", e));
        assert_eq!((0..50).map(pick).collect::<Vec<_>>(), (0..50).map(pick).collect::<Vec<_>>());
        // Every look gets drawn at some point (not stuck on one).
        let seen: HashSet<_> = (0..200).filter_map(pick).collect();
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn look_rules() {
        let original = donor("A", [0.0; 3], 7, "NMI", false, false);
        let near = |k: &str| donor(k, [500.0, 0.0, 0.0], 0, "NMI", false, false);
        let far = |k: &str| donor(k, [9000.0, 0.0, 0.0], 0, "NMI", false, false);
        let chosen = |donors: &[Donor], taken: &[String]| -> HashSet<String> {
            (0..200).filter_map(|e| choose_look(&original, donors, taken, &mut LookRng::new("M", "S", e)).map(|i| donors[i].look.key())).collect()
        };
        let set = |v: &[&str]| v.iter().map(|k| Look { mesh: Value::Asset((*k).into()), skins: Vec::new(), code_mesh: 0, timbre: None }.key()).collect::<HashSet<_>>();
        // Never the original's own look; nothing else: the original's look is kept.
        assert!(chosen(&[near("A")], &[]).is_empty());
        // Another alliance, another headgear or a role look is never lent (negative control:
        // the same donor fitting is taken).
        assert!(chosen(&[donor("B", [500.0, 0.0, 0.0], 0, "Allies", false, false)], &[]).is_empty());
        assert_eq!(chosen(&[donor("B", [500.0, 0.0, 0.0], 0, "nmi", false, false)], &[]), set(&["B"]));
        assert!(chosen(&[donor("B", [500.0, 0.0, 0.0], 0, "NMI", true, false)], &[]).is_empty());
        assert!(chosen(&[donor("B", [500.0, 0.0, 0.0], 0, "NMI", false, true)], &[]).is_empty());
        // An original with a role look keeps it.
        let role = donor("A", [0.0; 3], 0, "NMI", false, true);
        assert_eq!(choose_look(&role, &[near("B")], &[], &mut LookRng::new("M", "S", 0)), None);
        // Nearby (or same alert network) first, the whole map when nothing is near.
        assert_eq!(chosen(&[near("B"), far("C")], &[]), set(&["B"]));
        assert_eq!(chosen(&[donor("C", [9000.0, 0.0, 0.0], 7, "NMI", false, false), far("D")], &[]), set(&["C"]));
        assert_eq!(chosen(&[far("C"), far("D")], &[]), set(&["C", "D"]));
        // A look already given to another extra of the same original comes last.
        let b = near("B").look.key();
        assert_eq!(chosen(&[near("B"), near("C")], &[b.clone()]), set(&["C"]));
        assert_eq!(chosen(&[near("B")], &[b]), set(&["B"]));
        // Voices of a generator: only its listed ones.
        assert!((0..50).all(|e| timbre_of("13;", &mut LookRng::new("M", "S", e)).is_some_and(|t| t == 0 || t == 2)));
        assert_eq!(timbre_of("", &mut LookRng::new("M", "S", 0)), None);
    }
}