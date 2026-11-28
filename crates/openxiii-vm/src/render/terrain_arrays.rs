//! ULevel::UpdateTerrainArrays: ZoneInfo.Terrains and ULevel.Terrains.

use std::collections::HashMap;
use std::hash::Hash;

/// The arrays are rebuilt as follows: without a model, nothing changes. Otherwise the `Terrains` of the model's zones and those of the level
/// are emptied; each TerrainInfo of `actors` (in order, SetZone(1, 0) having set its zone `zone_of`) is added
/// without duplicates to its zone, to the level, then to each non-null zone of its ExtraZones; finally the `Terrains` of actor 0
/// (the LevelInfo) are emptied, even if a terrain was added there.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update<Id: Copy + Eq + Hash>(
    model: bool,
    model_zones: &[Option<Id>],
    actors: &[Option<Id>],
    is_terrain: impl Fn(Id) -> bool,
    is_zone: impl Fn(Id) -> bool,
    zone_of: impl Fn(Id) -> Id,
    extra_zones: impl Fn(Id) -> Vec<Option<Id>>,
    zones: &mut HashMap<Id, Vec<Id>>,
    level: &mut Vec<Id>,
) {
    if !model {
        return;
    }
    fn add_unique<Id: Copy + Eq>(v: &mut Vec<Id>, x: Id) {
        if !v.contains(&x) {
            v.push(x);
        }
    }
    for z in model_zones.iter().flatten() {
        zones.entry(*z).or_default().clear();
    }
    level.clear();
    for t in actors.iter().flatten().copied().filter(|&a| is_terrain(a)) {
        add_unique(zones.entry(zone_of(t)).or_default(), t);
        add_unique(level, t);
        for e in extra_zones(t).into_iter().flatten().filter(|&e| is_zone(e)) {
            add_unique(zones.entry(e).or_default(), t);
        }
    }
    if let Some(Some(a0)) = actors.first() {
        zones.entry(*a0).or_default().clear();
    }
}

