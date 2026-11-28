# Sounds, ambience and music

Replace a sound of the game on one map or everywhere, by another sound of the game or by your
own recording, and play a sound with a keyboard shortcut.

## Recipe

```toml
[map.Plage01.replace_sounds]
# the game's sound = the sound played instead (Sounds tab of the catalog)
"XIIIsound.Vehicles__PlgHelic.PlgHelic__hInit" = "XIIISound.Vehicles.KelloHelicoLoop1"

[sounds.MyHum]                  # your own file: "Mod.MyHum"
file = "sounds/my_hum.ogg"      # WAV or OGG, in the mod folder

[map.Banque01.replace_sounds]
"XIIIsound.Ambient.Generator1" = "Mod.MyHum"
```

On Plage01 the helicopter over the beach sounds like the one of another level; in the bank,
the generator hums with your own recording. Every other map keeps the game's sounds.

Names you can use here: on your PC, `Mods/_Guide/names/sounds.md` (sound effects),
`Mods/_Guide/names/ambient-sounds.md` (each map's ambience) and `Mods/_Guide/names/music.md`.

## Settings

| Block | Setting | What it does |
|---|---|---|
| `[map.<Map>.replace_sounds]` | `"game sound" = "new sound"` | on that map, the new sound plays each time the game plays the old one (ambient loops of the map too) |
| `[map."*".replace_sounds]` | same | on every map (`"Hual*"`: maps whose name starts with Hual) |
| `[sounds.<Name>]` | `file` | your own WAV or OGG file, then named `"Mod.<Name>"` |
| `[sounds.<Name>]` | `replace` | your sound plays instead of that game sound, on every map |
| `[[shortcut]]` | `sound` | a keyboard shortcut that plays a sound at the player (see [keyboard-shortcuts.md](keyboard-shortcuts.md)) |

The ambient sounds of each map (the loops its actors play) are listed in `maps.md`
(`oxiii mods-reference`). Sounds of other levels work anywhere: their sound bank is loaded
with the level when needed.

**Music.** The catalog's Music tab lists the music of the levels (the `Music…` groups of
`XIIISound`). The game plays its music through its own music system, driven by the level's
scripts; replacing a music sound by name with `replace_sounds` has not been checked on every
level yet, so try it and look at `mods-log.txt`.

## If it does not work

- The sound name must be complete: `Package.Group.Name` (copy it from the catalog).
- `mods-log.txt` says when a file cannot be read or a name is unknown.
- A replaced sound keeps the original's length: a longer recording is cut where the game's
  sound ended.
- Your sound files are shared in network games like the other files of the mod: everyone
  needs the same mods.

All the details (sound banks, variants, timbres): [expert/sounds.md](expert/sounds.md).
