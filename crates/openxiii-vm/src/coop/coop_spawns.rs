//! Co-op: chosen starting places of the second player (improved build only, through co-op). By default the second player starts at the first free place next
//! to the first one (`coop_places_near`). For a map start or a checkpoint listed in `coop_spawns.tsv`, the second player starts at the place and facing
//! written there instead (falling back to the free place next to the first player if that place is taken). An entry is found by the map, the portal of the
//! travel (the checkpoint's start, empty for the map's own start) and where the first player stood when the second one logged in: two checkpoints of one map
//! with the same portal still differ by their place.

use crate::value::{ObjId, Value};
use crate::Vm;

/// Built-in table (same format as the recorded file).
const BUILT_IN: &str = include_str!("coop_spawns.tsv");

/// Header written at the top of a new recorded file.
const HEADER: &str = "# Co-op: starting place of the second player, per map start or checkpoint.\n\
# map\tportal\tfirst_x\tfirst_y\tfirst_z\tsecond_x\tsecond_y\tsecond_z\tpitch\tyaw\troll\tcrouched\n\
# (optional, a start with room for one: where the first player is moved first)\tfirst_to_x\tfirst_to_y\tfirst_to_z\tpitch\tyaw\troll\tcrouched\n";

/// How far the first player may stand from a recorded entry's first place for it to apply.
const SAME_START: f32 = 256.0;

/// One start of the table.
#[derive(Debug, Clone, PartialEq)]
pub struct CoopSpawn {
    /// Map name, without its extension.
    pub map: String,
    /// Portal of the travel ("" for the map's own start).
    pub portal: String,
    /// Where the first player stood when the second one logged in.
    pub first: [f32; 3],
    /// Second player's place and facing (controller rotation).
    pub second: [f32; 3],
    pub rotation: [i32; 3],
    /// Second player crouched.
    pub crouched: bool,
    /// A start with room for one player (the duct of PRock03): where the first player is moved
    /// first, so that the second fits where the first one started.
    pub first_to: Option<Place>,
}

/// A player's place, facing (controller rotation) and crouching.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub at: [f32; 3],
    pub rotation: [i32; 3],
    pub crouched: bool,
}

/// Where the first player started on this map: map, portal, place.
#[derive(Debug, Clone, Default)]
pub struct SpawnAnchor {
    pub map: String,
    pub portal: String,
    pub first: [f32; 3],
}

/// Map name without its extension or options.
pub fn map_key(map: &str) -> String {
    let map = map.split(['?', '#']).next().unwrap_or(map);
    map.rsplit_once('.').map_or(map, |(m, _)| m).trim().to_string()
}

/// Parses the table: one start per line, tab-separated; "#" starts a comment line.
pub fn parse(text: &str) -> Vec<CoopSpawn> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 11 {
            continue;
        }
        let num = |i: usize| f[i].trim().parse::<f32>().ok();
        let int = |i: usize| f[i].trim().parse::<i32>().ok();
        let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(g)) = (num(2), num(3), num(4), num(5), num(6), num(7)) else { continue };
        let (Some(p), Some(y), Some(r)) = (int(8), int(9), int(10)) else { continue };
        let crouched = f.get(11).is_some_and(|c| c.trim() == "1");
        let first_to = (f.len() >= 19)
            .then(|| {
                let v: Vec<f32> = f[12..15].iter().filter_map(|x| x.trim().parse().ok()).collect();
                let r: Vec<i32> = f[15..18].iter().filter_map(|x| x.trim().parse().ok()).collect();
                (v.len() == 3 && r.len() == 3).then(|| Place { at: [v[0], v[1], v[2]], rotation: [r[0], r[1], r[2]], crouched: f[18].trim() == "1" })
            })
            .flatten();
        out.push(CoopSpawn { map: map_key(f[0]), portal: f[1].trim().to_string(), first: [a, b, c], second: [d, e, g], rotation: [p, y, r], crouched, first_to });
    }
    out
}

fn line_of(s: &CoopSpawn) -> String {
    let mut line = format!(
        "{}\t{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{}\t{}\t{}\t{}",
        s.map, s.portal, s.first[0], s.first[1], s.first[2], s.second[0], s.second[1], s.second[2], s.rotation[0], s.rotation[1], s.rotation[2], u8::from(s.crouched)
    );
    if let Some(m) = s.first_to {
        line += &format!(
            "\t{:.1}\t{:.1}\t{:.1}\t{}\t{}\t{}\t{}",
            m.at[0], m.at[1], m.at[2], m.rotation[0], m.rotation[1], m.rotation[2], u8::from(m.crouched)
        );
    }
    line
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The entry of `table` for that start: same map and portal, nearest first place within
/// [`SAME_START`].
pub fn find<'a>(table: &'a [CoopSpawn], anchor: &SpawnAnchor) -> Option<&'a CoopSpawn> {
    let map = map_key(&anchor.map);
    table
        .iter()
        .filter(|s| s.map.eq_ignore_ascii_case(&map) && s.portal.eq_ignore_ascii_case(&anchor.portal))
        .map(|s| (distance(s.first, anchor.first), s))
        .filter(|(d, _)| *d <= SAME_START)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| s)
}

/// `text` with `spawn` recorded: the line of the same start (same map, portal and first place
/// within [`SAME_START`]) replaced, other lines and comments kept.
pub fn record(text: &str, spawn: &CoopSpawn) -> String {
    let mut out = if text.trim().is_empty() { HEADER.to_string() } else { String::new() };
    let mut replaced = false;
    for line in text.lines() {
        let same = parse(line).first().is_some_and(|s| {
            s.map.eq_ignore_ascii_case(&spawn.map) && s.portal.eq_ignore_ascii_case(&spawn.portal) && distance(s.first, spawn.first) <= SAME_START
        });
        if same {
            if !replaced {
                out.push_str(&line_of(spawn));
                out.push('\n');
                replaced = true;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !replaced {
        out.push_str(&line_of(spawn));
        out.push('\n');
    }
    out
}

impl Vm {
    /// No colliding pawn's cylinder overlaps a pawn of this size standing at `place` (the
    /// placed pawn itself is never there yet).
    pub(crate) fn coop_place_free(&self, place: [f32; 3], radius: f32, height: f32) -> bool {
        self.coop_place_free_but(place, radius, height, &[])
    }

    /// Same, not counting the pawns `but` (the ones being placed).
    pub(crate) fn coop_place_free_but(&self, place: [f32; 3], radius: f32, height: f32, but: &[ObjId]) -> bool {
        !self.actors.iter().any(|&a| {
            if but.contains(&a) || !self.objects.get(a).is_some_and(|o| o.is_some()) || !self.is_a(a, "Pawn") || !self.prop(a, "bCollideActors").as_bool() {
                return false;
            }
            let at = self.prop(a, "Location").as_vector();
            let (r, h) = (self.prop(a, "CollisionRadius").as_float(), self.prop(a, "CollisionHeight").as_float());
            // The boxes around the two cylinders, not the cylinders: the engine's encroachment
            // check on a move (physics.rs) compares those, and a pawn moved into another one's
            // box kills it (SMarin02: 55 units apart on each axis, 78 in distance).
            #[cfg(test)]
            if self.coop.place_cylinders {
                return ((at[0] - place[0]).powi(2) + (at[1] - place[1]).powi(2)).sqrt() < r + radius && (at[2] - place[2]).abs() < h + height;
            }
            (at[0] - place[0]).abs() < r + radius && (at[1] - place[1]).abs() < r + radius && (at[2] - place[2]).abs() < h + height
        })
    }

    /// A second player waiting on a start with room for one (coop_after_extra_login): their
    /// collision comes back once nobody else stands where they are.
    pub(crate) fn coop_spawn_wait_frame(&mut self) {
        let Some((pawn, flags)) = self.coop.spawn_waiting else { return };
        if !self.coop_object_alive(pawn) {
            self.coop.spawn_waiting = None;
            return;
        }
        let at = self.prop(pawn, "Location").as_vector();
        let (radius, height) = (self.prop(pawn, "CollisionRadius").as_float(), self.prop(pawn, "CollisionHeight").as_float());
        if !self.coop_place_free_but(at, radius, height, &[pawn]) {
            return;
        }
        self.coop.spawn_waiting = None;
        let _ = self.call_named(pawn, "SetCollision", flags.iter().map(|&b| Value::Bool(b)).collect());
        self.log.push("Log: Co-op: the second player has room now".to_string());
    }

    /// The pawn is in water (its physics volume is a water volume, or it swims).
    pub(crate) fn coop_in_water(&self, pawn: ObjId) -> bool {
        const PHYS_SWIMMING: u8 = 3;
        let volume = self.prop(pawn, "PhysicsVolume").as_object().filter(|&v| self.objects.get(v).is_some_and(|o| o.is_some()));
        volume.is_some_and(|v| self.prop(v, "bWaterVolume").as_bool()) || matches!(self.prop(pawn, "Physics"), Value::Byte(PHYS_SWIMMING))
    }

    /// The table in use: the recorded file of a recording build first, then the built-in one.
    fn coop_spawn_table(&self) -> Vec<CoopSpawn> {
        #[allow(unused_mut)]
        let mut table = Vec::new();
        table.extend(parse(BUILT_IN));
        table
    }

    /// After the second player's login: the start is remembered and, when the table has an
    /// entry for it, the second pawn goes there. Returns false when it was not placed.
    pub(crate) fn coop_spawn_place(&mut self, first: ObjId, pawn: ObjId, pc: ObjId) -> crate::Result<bool> {
        let at = self.prop(first, "Location").as_vector();
        if self.coop.spawn_anchor.is_none() {
            self.coop.spawn_anchor = Some(SpawnAnchor { map: map_key(&self.url.map), portal: self.coop.spawn_portal.clone(), first: at });
        }
        let anchor = SpawnAnchor { map: map_key(&self.url.map), portal: self.coop.spawn_portal.clone(), first: at };
        // A start in water (the first player sinks): the second one goes beside them at the same
        // height (coop_places_near), never at a place recorded once they had sunk.
        if self.coop_in_water(first) {
            return Ok(false);
        }
        let Some(spawn) = find(&self.coop_spawn_table(), &anchor).cloned() else { return Ok(false) };
        // Room for one: the first player moves on first (both pawns being placed are not
        // counted, the second one still stands on the first one's start).
        if let Some(to) = spawn.first_to {
            let Some(first_pc) = self.coop.players.first().copied() else { return Ok(false) };
            if !self.coop_put(first, first_pc, to, &[first, pawn])? {
                self.log.push("Log: Co-op: the first player's place of this start is taken".to_string());
                return Ok(false);
            }
        }
        let second = Place { at: spawn.second, rotation: spawn.rotation, crouched: spawn.crouched };
        if !self.coop_put(pawn, pc, second, &[pawn])? {
            self.log.push(format!("Log: Co-op: chosen start of the second player taken ({:.0},{:.0},{:.0})", spawn.second[0], spawn.second[1], spawn.second[2]));
            return Ok(false);
        }
        self.log.push(format!("Log: Co-op: second player at the chosen start {:.0},{:.0},{:.0}", spawn.second[0], spawn.second[1], spawn.second[2]));
        Ok(true)
    }

    /// Puts a player's pawn at a recorded place, facing and crouching as recorded. False when
    /// the place is taken or does not fit.
    fn coop_put(&mut self, pawn: ObjId, pc: ObjId, to: Place, but: &[ObjId]) -> crate::Result<bool> {
        let place = to.at;
        let radius = self.prop(pawn, "CollisionRadius").as_float();
        let height = if to.crouched { self.prop(pawn, "CrouchHeight").as_float() } else { self.prop(pawn, "CollisionHeight").as_float() };
        if !self.coop_place_free_but(place, radius, height, but) {
            return Ok(false);
        }
        // Moved without touching anybody (the pawn may still stand inside the first one, on
        // the start they share: any move would crush them); the place is free (just above).
        let flags = ["bCollideActors", "bBlockActors", "bBlockPlayers"].map(|n| self.prop(pawn, n).as_bool());
        self.call_named(pawn, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)])?;
        // A crouched place is the centre of a crouched pawn, maybe where only a crouched pawn
        // fits (a duct): the pawn crouches first (APawn::Crouch, smaller cylinder).
        if to.crouched && self.prop(pawn, "bCanCrouch").as_bool() {
            self.crouch(pawn, false);
        }
        let moved = self.call_named(pawn, "SetLocation", vec![Value::vector(place[0], place[1], place[2])])?.as_bool();
        self.call_named(pawn, "SetCollision", flags.iter().map(|&b| Value::Bool(b)).collect())?;
        if !moved {
            return Ok(false);
        }
        let [p, y, r] = to.rotation;
        self.call_named(pawn, "SetRotation", vec![Value::rotator(0, y, 0)])?;
        self.set_prop(pc, "Rotation", Value::rotator(p, y, r));
        // Crouched: the toggle of the player's input (iDuckMem, player_input.rs) and the pawn.
        if to.crouched && self.prop(pawn, "bCanCrouch").as_bool() {
            if let Some(input) = self.prop(pc, "PlayerInput").as_object() {
                self.set_prop(input, "iDuckMem", Value::Int(1));
            }
            self.set_prop(pc, "bDuck", Value::Byte(1));
            self.call_named(pawn, "ShouldCrouch", vec![Value::Bool(true)])?;
        }
        Ok(true)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn(map: &str, portal: &str, first: [f32; 3], second: [f32; 3]) -> CoopSpawn {
        CoopSpawn { map: map.into(), portal: portal.into(), first, second, rotation: [0, 16384, 0], crouched: false, first_to: None }
    }

    #[test]
    fn record_then_find() {
        let a = spawn("Plage01", "", [0.0, 0.0, 0.0], [100.0, 0.0, 0.0]);
        let b = spawn("Plage01", "Checkpoint2", [5000.0, 0.0, 0.0], [5100.0, 0.0, 0.0]);
        let text = record(&record("", &a), &b);
        assert!(text.starts_with('#'));
        let table = parse(&text);
        assert_eq!(table.len(), 2);
        let at = |map: &str, portal: &str, first| SpawnAnchor { map: map.into(), portal: portal.into(), first };
        assert_eq!(find(&table, &at("plage01.unr", "", [10.0, 0.0, 0.0])), Some(&a));
        assert_eq!(find(&table, &at("Plage01", "checkpoint2", [5000.0, 0.0, 0.0])), Some(&b));
        // Another place, another portal or another map: nothing.
        assert_eq!(find(&table, &at("Plage01", "", [2000.0, 0.0, 0.0])), None);
        assert_eq!(find(&table, &at("Plage01", "Other", [0.0, 0.0, 0.0])), None);
        assert_eq!(find(&table, &at("Banque01", "", [0.0, 0.0, 0.0])), None);
        // A new recording of the same start replaces the old line.
        let to = Place { at: [80.0, 0.0, 0.0], rotation: [0, 100, 0], crouched: true };
        let a2 = CoopSpawn { crouched: true, first_to: Some(to), ..spawn("Plage01", "", [3.0, 0.0, 0.0], [-100.0, 50.0, 0.0]) };
        let table = parse(&record(&text, &a2));
        assert_eq!(table.len(), 2);
        assert_eq!(find(&table, &at("Plage01", "", [0.0, 0.0, 0.0])), Some(&a2));
    }

    #[test]
    fn built_in_table_parses() {
        let lines = BUILT_IN.lines().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).count();
        assert_eq!(parse(BUILT_IN).len(), lines);
    }
}
