# Add a weapon

Make a new weapon of your own, beside the game's: it works like a weapon of the game you
choose, with its own name, values, sound, model and icon.

## Recipe

```toml
[classes.Blaster]          # a new weapon "Mod.Blaster"
parent = "XIII.Beretta"    # it works like the 9mm pistol

[classes.BlasterPick]      # the pickup that gives it
parent = "XIII.BerettaPick"

[class."Mod.Blaster"]
ItemName = "Blaster"
ReloadCount = 30
PickupClassName = "Mod.BlasterPick"

[class."Mod.BlasterPick"]
InventoryType = "Mod.Blaster"
```

Then give it to XIII (`[map.Plage01.start] weapons = ["Mod.Blaster"]`) or put its pickup on a
map (`[[map.Plage01.add]] class = "Mod.BlasterPick"`, see
[add-an-object-or-decoration.md](add-an-object-or-decoration.md)).

Names you can use here: `Mods/_Guide/names/weapons-and-ammo.md` on your PC (the weapon classes to
start from, their pickups and ammo), and [names/words.md](names/words.md) (fire modes).

## Settings

| Setting | Block | What it does |
|---|---|---|
| `parent` | `[classes.<Name>]` | the game class it works like (Weapons tab, Class column) |
| `ItemName`, `HudIcon` | weapon | its name, its icon (a picture of the mod, 64 × 32) |
| `AmmoName` | weapon | its ammo class (a game ammo, or a new one made the same way) |
| `hFireSound` | weapon | its firing sound (a sound of the game or of the mod) |
| `FirstPersonModel` | weapon | the model held in first person |
| `StaticMesh` | pickup | the model lying on the ground |

A new weapon gets a weapon slot of its own (the number keys and the wheel find it); the log
says which. A complete weapon is four classes: the weapon, its pickup, the gun seen in a
character's hand (attachment) and its ammo. The example mod `examples/mods/blaster-gun` has all
four, with its own model, texture, sound, tracers and impacts.

## If it does not work

- `mods-log.txt` says "unknown parent" or "name taken": the parent must be a class of the game
  (Class column of the Weapons tab), and the new name must not be one.
- The weapon has the parent's ammo: set `AmmoName`.
- The pickup gives the parent weapon: set its `InventoryType`.
- Everyone in a network game must have the same mods (see
  [multiplayer-and-coop.md](multiplayer-and-coop.md)).

All the details (attachments, models, tracers, impacts, sparks):
[expert/new-classes.md](expert/new-classes.md) and
[expert/textures-and-models.md](expert/textures-and-models.md).
