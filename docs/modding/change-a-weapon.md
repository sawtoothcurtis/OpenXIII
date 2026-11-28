# Change a weapon

Change how a weapon of the game behaves: its magazine, its damage, its accuracy, how fast it
fires and reloads.

## Recipe

```toml
[class."XIII.Beretta"]     # the 9mm pistol (Class column of the catalog's Weapons tab)
ReloadCount = 17           # rounds in a magazine (the game: 13)
PickupAmmoCount = 17       # rounds in it when picked up: raise both together
FireAnimRate = 1.5         # fires 1.5 times as fast

[class."XIII.c9mmAmmo"]    # its ammo (Ammo class column)
SRDamages = 35             # damage at short range (the game: 25)
```

Every weapon of the game changes, everywhere. To change it on one map only, write the table as
`[map.Plage01.class."XIII.Beretta"]`.

Names you can use here: [names/weapons.md](names/weapons.md), and on your PC
`Mods/_Guide/names/weapons-and-ammo.md` (every weapon class with its ammo class).

## Settings

| Setting | Block | What it does |
|---|---|---|
| `ReloadCount`, `PickupAmmoCount` | weapon | magazine size, rounds when picked up |
| `FireAnimRate`, `ReloadAnimRate`, `SelectAnimRate` | weapon | speed of firing, reloading and taking it out (1.0 = the game's) |
| `TraceAccuracy` | weapon | spread of the shots: smaller is more accurate |
| `SRDamages`, `LRDamages` | ammo | damage at short and long range |
| `MaxAmmo` | ammo | most rounds XIII can carry |
| `ItemName` | weapon | the name shown in the game |

The player's rate of fire is not a number of the game: the next shot comes when the firing
animation ends, so `FireAnimRate` is the way to change it (`ShotTime` only matters for the
enemies). To give the weapon to XIII when a level starts:

```toml
[map.Plage01.start]
weapons = ["Pistol", "Shotgun"]   # simple names of the Weapons tab, or classes
```

## If it does not work

- The bigger magazine only shows after the first reload: raise `PickupAmmoCount` too.
- `mods-log.txt` lists every value applied, with its file and line; an unknown class or
  property gives one warning and is skipped.
- Two mods that set the same value: the one whose folder comes last wins, and the Mods page
  shows the conflict in red.
- The full list of each weapon's properties and their original values is in `weapons.md`
  (`oxiii mods-reference`, see [find-names.md](find-names.md)).

All the details (fire modes, how a weapon is held, another weapon's look in first person):
[expert/weapons.md](expert/weapons.md).
