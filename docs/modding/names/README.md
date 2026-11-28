# All the names a mod can use

Every name a mod writes comes from one of these lists. Open a list, search it with Ctrl+F,
copy the name from the first column and paste it into your `.toml` file.

**Here:** the lists OpenXIII defines itself.

| List | What it lists | Used by the settings | Example |
|---|---|---|---|
| [characters.md](characters.md) | Characters (simple names) and voices | `character`, `look`, `spawn`, `voice` | `character = "Jones"` |
| [weapons.md](weapons.md) | Weapons (simple names) | `weapon`, `weapons` | `weapon = "Shotgun"` |
| [words.md](words.md) | Fixed words: team, behaviour, when, events and actions of reactions, map selectors, key names, fire modes, commands | many settings | `team = "enemy"` |

**On your PC:** the lists made from your own copy of XIII. The game writes them by itself
into the `_Guide` folder of your `Mods` folder (when it starts, if they are missing or
out of date; delete them to have them written again): the names
catalog `Modding catalog.html` (one page with a search box) and these lists in
`Mods/_Guide/names/`. See [find-names.md](../find-names.md).

| List (created on your PC) | What it lists | Used by the settings | Example |
|---|---|---|---|
| `Mods/_Guide/names/weapons-and-ammo.md` | Every weapon class, its ammo and its pickup | `give`, `[class."…"]` | `give = "XIII.m16"` |
| `Mods/_Guide/names/objects.md` | Pickups and objects a mod can place | `class` of `[[map.<Map>.add]]` | `class = "XIII.MedPick"` |
| `Mods/_Guide/names/decorations.md` | Solid models (static meshes) | `model` of `[[map.<Map>.add]]` | `model = "Bateau_pack.BAT_caisse01"` |
| `Mods/_Guide/names/maps.md` | Maps, checkpoints, and the characters each map places | `[map.<Map>…]`, `from_checkpoint`, `who`, `by` | `who = "gardien_lostpass"` |
| `Mods/_Guide/names/events.md` | Events each map triggers | `event` of `[[map.<Map>.say]]` | `event = "Objectif91"` |
| `Mods/_Guide/names/dialogues.md` | Dialogue lines (name, speaker, level) | `line` of `[[map.<Map>.say]]` | `line = "Plage01_Pam_00"` |
| `Mods/_Guide/names/sounds.md` | Sound effects, by group | `sound`, `[map.<Map>.replace_sounds]` | `sound = "XIIISound.Doors.Open"` |
| `Mods/_Guide/names/ambient-sounds.md` | The sounds each map plays as ambience | `[map.<Map>.replace_sounds]` | `"XIIIsound.Ambient.Generator1" = "Mod.MyHum"` |
| `Mods/_Guide/names/music.md` | Music of the levels | `[map.<Map>.replace_sounds]`, `sound` | `"XIIISound.Music.Banque01"` |
| `Mods/_Guide/names/animations.md` | Animations of characters and weapons | `animation` of `[character.<Name>.state.<State>]` | `animation = "WaitNeutre"` |
| `Mods/_Guide/names/scripts.md` | Events and actions of when blocks, events and functions of scripts | `[[map.<Map>.when]]`, the `scripts` folder | `{ give = "Shotgun" }` |

The lists made on your PC hold identifiers only (no dialogue text, no subtitle); they are
not part of the repository.
