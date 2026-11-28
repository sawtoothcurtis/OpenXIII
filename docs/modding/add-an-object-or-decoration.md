# Add an object or a decoration

Put a pickup (a weapon, ammo, a med kit) or a solid decoration on a map, or take away one the
map places.

## Recipe

```toml
[[map.Plage01.add]]
class = "XIII.FusilSnipePick"          # a pickup: Objects and pickups tab
position = [-450, -912, 1190]          # a point on the ground (console command modpos)

[[map.Plage01.add]]
model = "Bateau_pack.BAT_caisse01"      # a decoration: Decorations tab
position = [-6643, -11524, 690]
scale = 1.5
```

In the game, walk to the spot, face the way the object should face, press **F2** and type
`modpos`: the game prints the `position` and the `yaw` to paste.

Names you can use here: on your PC, `Mods/_Guide/names/objects.md` (pickups and objects) and
`Mods/_Guide/names/decorations.md` (solid models); [names/words.md](names/words.md) (map selectors).

## Settings

| Setting | What it does | Example |
|---|---|---|
| `class` or `model` | what to add: a class (pickups, objects) or a static mesh (decorations) | `class = "XIII.MedPick"` |
| `yaw`, `rotation` | facing in degrees, or `[pitch, yaw, roll]` | `rotation = [0, 45, 0]` |
| `scale` | its size: a number, or `[x, y, z]` | `scale = 2.0` |
| `collision` | `true`: it blocks players and shots; `false`: everything goes through | `collision = true` |
| `from_checkpoint` | only when the level starts from that checkpoint (Maps tab) | `from_checkpoint = 2` |

To take away or swap what the map places:

```toml
[[map.Plage01.remove]]
name = "BerettaPick0"                  # an actor of the map, by its name

[map.Plage01.replace]                  # every actor of this class becomes another
"XIII.TKnifePick" = "XIII.MagnumPick"
```

## If it does not work

- Nothing appears: `mods-log.txt` says why (unknown class or model, position outside the map,
  no room). Take the position again with `modpos`.
- A decoration floats or sinks: its origin is not always at its base; lower or raise the third
  number of `position`.
- A pickup cannot be picked up when `collision = true`: leave `collision` out for pickups.
- Your own 3D model instead of a game mesh: see
  [expert/textures-and-models.md](expert/textures-and-models.md).

All the details (selectors such as `"Hual*"`, start weapons, replacing): [expert/placing.md](expert/placing.md).
