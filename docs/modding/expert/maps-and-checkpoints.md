# Maps and checkpoints

Changes made only on one map or from one checkpoint, and the selectors of map entries. Back to [the modding guide](../README.md).

## Maps and checkpoints

`maps.md` lists every map of the game: the 34 solo maps in the order of the story (from
`Plage00` to `Bateau01`, each map's `MapInfo` naming the next), the multiplayer maps (`DM_`,
`CTF_`, `SB_`) and the technical ones (menu, credits). For each solo map it gives its level
title, where the player starts and its save points.

A story level is often made of several maps (`Hual01a`, then `Hual01b`): each is a map of its
own for the mods. Inside a map, the game saves at **save points**. The game counts the saves of a
level (`XIIIGameInfo.CheckpointNumber`): the save made when the map is entered is 1, the
checkpoints reached in the map are 2, 3… `maps.md` numbers them that way. Loading a checkpoint
loads the map again from its file and starts the player at the save point; the mods' map
changes are made again each time.

**Several maps at once.** Wherever a map name goes (`[map.<Map>.class."..."]`, `add`, `remove`,
`replace`, `start`, `load_sound_banks`, `load_packages`), these selectors can be used, in quotes:

| Selector | Maps |
|---|---|
| `"Plage01"` | that map |
| `"Hual*"` | the maps whose name starts with `Hual` (Hual01a, Hual01b, Hual02, Hual04a, Hual04c) |
| `"solo"` | the maps of the story (not the multiplayer maps, the menu or the credits) |
| `"multi"` | the multiplayer maps (names starting with `DM_`, `CTF_` or `SB_`) |
| `"*"` | every map |

```toml
[map."solo".start]                 # a pistol at the start of every map of the story
weapons = ["XIII.Beretta"]

[map."Hual*".class."XIDPawn.BaseSoldier"]
Health = 250                       # the soldiers of the Hualpar maps

[map."SPADS*".class."XIII.Beretta"]
FireAnimRate = 1.5
```

When values for the same map and property come from several selectors, the most precise one
wins: the map's own name, then the longest beginning of name (`"Hual01*"` over `"Hual*"`), then
`"solo"` or `"multi"`, then `"*"`; the order in the files does not matter. The actors added,
removed and given by `start` add up; two `replace` of the same class: the most precise selector's
wins.

**Checkpoints.** An `add` or `remove` entry can be made only when the level starts from a given
checkpoint or a later one, and start weapons can be given only when it starts from one
checkpoint, with the numbers of `maps.md`:

```toml
[[map.Banque01.add]]
class = "XIII.MedPick"
position = [-2560, 100, 1047]
from_checkpoint = 3                # only when Banque01 starts from checkpoint 3 or later

[[map.Banque01.remove]]
name = "MedPick0"
from_checkpoint = 2

[map.Banque01.checkpoint.3.start]  # given only when Banque01 starts from checkpoint 3
weapons = ["XIII.m16"]
ammo = { "XIII.M16Ammo" = 60 }
```

- The checkpoint is the one the level is loaded from: a checkpoint is loaded at its save point's
  teleporter (`Banque01#Teleporter_otage`), which `maps.md` gives. Starting the map from its
  beginning, or entering it from the previous map, is checkpoint 1.
- The changes without a condition are made whatever the checkpoint; `[map.<Map>.start]` is given
  at every start, `[map.<Map>.checkpoint.<N>.start]` adds to it at checkpoint N. Selectors work
  too (`[map."solo".checkpoint.2.start]`: the first checkpoint of every map of the story).
- The number of a save point is its place in the map's order (../maps.md). A player who skipped a
  checkpoint saves the next one under a lower number in the game's own counter: the mods use the
  maps.md number, that of the save point loaded.
