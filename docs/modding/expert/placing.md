# Placing, removing and replacing actors

Every key of `[[map.<Map>.add]]`, `remove`, `replace` and `start`, and how to find positions. Back to [the modding guide](../README.md).

## Placing things on maps

A mod can add actors to a map (pickups, weapons, decorations, characters), remove or replace the
ones the map placed, and give weapons to the player when a level starts. Map names are the map
file names without `.unr` (`Plage01`, `Banque01`…).

```toml
[[map.Plage01.add]]                    # one entry per actor
class = "XIII.FusilSnipePick"          # full class name (../weapons.md, characters.md)
position = [-450, -912, 1190]          # point on the ground where the actor stands
yaw = 90                               # facing, in degrees (optional)
properties = { Health = 200 }          # values of this actor only (optional)

[[map.Plage01.add]]                    # a solid decoration: a static mesh, instead of a class
model = "Mod.Crate"                    # a mod's model, or a game mesh "Package.Group.Name"
position = [430, -2480, 992]           # on the pontoon, at the bottom of the hut's stairs
properties = { DrawScale = 2.0 }       # optional, as above

[[map.Plage01.remove]]                 # an actor placed by the map, by its name…
name = "BerettaPick0"

[[map.Plage01.remove]]                 # …or the actors of a class near a point
class = "XIII.MedPick"
near = [-416, -780, 1289]
radius = 200                           # optional, 100 by default

[map.Plage01.replace]                  # every actor of exactly this class becomes another
"XIII.TKnifePick" = "XIII.MagnumPick"

[map.Plage01.start]                    # given to the player when the level starts
weapons = ["XIII.Beretta", "XIII.FusilChasse"]
ammo = { "XIII.c9mmAmmo" = 30 }        # optional: ammo amount afterwards (at most MaxAmmo)
```

**Turning, sizing and solidity.** Any added actor (a class or a `model`) also takes:

| Key | Write | What it does |
|---|---|---|
| `rotation` | `[pitch, yaw, roll]` in degrees | turns it: pitch raises its front, yaw turns it clockwise seen from above (as `yaw` alone, which it replaces), roll tilts it sideways |
| `scale` | a number, or `[x, y, z]` | its size: `1.5` is half as big again; `[1, 1, 2]` twice as tall (along its own X, Y, Z) |
| `collision` | `true` or `false` | `true`: players, characters and shots bump into it; `false`: everything goes through it. Without it the actor keeps its class's own (a decoration added with `model` is solid; a pickup is not, so that it can be picked up) |

```toml
[[map.Plage01.add]]                    # a crate in the cabin, turned and bigger, that blocks
model = "Mod.Crate"
position = [-500, -872, 1190]
rotation = [0, 45, 0]                  # a corner towards the door
scale = 1.5
collision = true

[[map.Plage01.add]]                    # the blaster on the floor of the beach hut
class = "Mod.BlasterPick"
position = [-470, -860, 1190]
rotation = [0, 90, 0]
```

`properties` are written after these keys and win over them (`properties = { DrawScale = 2.0 }`
still works).

**Finding positions.** Play the level in a mods build and walk to the exact spot where the
actor should stand, facing the way it should face. Press **F2** (the game's console key) and
type `modpos`, then Enter. It prints the map name, the point on the ground under the player and
the direction the player faces, ready to paste into an `[[map.<Map>.add]]` entry:

```toml
# Plage01
position = [-742, -808, 1121]
yaw = 270
```

The same lines are written to `mods-log.txt` (lines starting with `modpos:`), so you can copy them
from there. To place several things near each other, stand at each place in turn and type
`modpos` each time.

How positions, angles and sizes work:

- **Units.** One game unit is about 2 cm (50 units per metre, the default `scale` of a mod's
  model). Two pickups 50 to 60 units apart lie side by side without touching.
- **Axes.** `[x, y, z]`: X and Y are the two directions on the ground, Z is up. The level designer
  chose where X and Y point on each map, so use `modpos` rather than guessing: walk a few steps,
  type it again, and see which number changes.
- **`yaw`** is in degrees, turning clockwise seen from above: `0` faces +X, `90` faces +Y, `180`
  faces -X, `270` faces -Y. `rotation = [pitch, yaw, roll]` adds the tilts (in degrees too).
- **Height.** `position` is a point on the floor: the game raises the actor by its own collision
  height so that it stands on it. Keep the Z that `modpos` gives for that floor; if there is
  no room for the actor there, it is not added and the log says so.
- **Size.** `scale = 1.5` makes it half as big again; a mod's own model is measured in glTF metres
  times its `scale` key (50 game units per metre by default). Actor names, for `remove`, are the names the level
designer gave (`BerettaPick0`, `MedPick2`…); the log of a level start with
`OXIII_MODS_DUMP=1` lists the characters by name.

How it works, and what to expect:

- **Added actors** are created by the engine itself when the level starts, after the map's own
  actors and before the first frame, exactly as the game creates actors during play. A
  character gets its AI like the characters placed in the map, and reacts to the player
  normally: it stays where it is put or follows the map's paths, but cannot be given a route of
  its own yet. `properties` are written before the actor starts, so they count from the first
  moment. The actor's centre is raised by its collision height so that it stands on
  `position`; if there is no room there, or the position is outside the map, a warning says so
  and nothing is added.
- **Decorations** (`model = "…"`) are actors of the class `Mod.ModelDecoration`, made from the
  game's `Engine.StaticMeshActor` (the class of the map's own decorations) so that they can be
  added during play. They are part of the world for the game: they block the player, the
  characters, the shots and the line of sight, and do not move or fall. What blocks is chosen by
  the model's `collision` (a game mesh collides as in the game). The model's origin stands on
  `position`. `Mod.ModelDecoration` is a class name of its own: a mod cannot make another class
  with that name.
- **Removed actors** are taken out before any script of the level runs, the way the level
  editor's deleted actors are. Actors that are part of the level's geometry (`bStatic`) cannot
  be removed; a warning says so. Removing an actor that the level's scripts need (a trigger, a
  door, a character of a cutscene) can block the level.
- **Replaced actors** are removed, and an actor of the new class is created at the same place,
  with the same rotation and the same `Tag` and `Event`, so triggers still find it. Only actors
  of exactly that class are replaced, not subclasses.
- **Start weapons** are given with the game's own `GiveWeapon`, which skips a weapon the player
  already has, with the ammo its pickup gives. They are given each time the level starts,
  including after a checkpoint.
- **Saved games.** XIII saves checkpoints: loading one loads the level again from its map, then
  restores the player's inventory. The changes are made again at each load, so added actors
  are never doubled nor lost, and removed ones stay removed.
- Unknown class, unknown actor name, wrong position: one warning line in the log, never a
  crash. The log also lists every change made, for example
  `mods: Plage01: add XIII.FusilSnipePick at [-450, -912, 1190]: FusilSnipePick0 at [-450, -912, 1200] (…)`.

A complete example is in [examples/mods/beach-weapons](../../../examples/mods/beach-weapons/mod.toml).
