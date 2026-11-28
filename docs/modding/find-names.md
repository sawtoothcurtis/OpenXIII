# Find names

Every name a mod uses (a character, a weapon, a map, a sound, a line) comes from a list.
**Start with [names/](names/README.md)**: the lists of characters, weapons, voices and fixed
words are right there in the documentation; the lists made from the game are on your PC.

## Recipe

1. Read [names/characters.md](names/characters.md) and [names/weapons.md](names/weapons.md)
   for the simple names (Jones, Carrington, Shotgun…).
2. For everything else (maps and their characters, sounds, music, dialogue lines, objects,
   decorations, animations, events), open the `_Guide` folder of your `Mods` folder.
   The game writes it by itself when it starts, if it is missing or out of date:
   - `Modding catalog.html`: every list in one page with a search box (open it in any web
     browser, it works offline), and a **Copy** button on each row;
   - `names/`: the same lists as plain pages.
3. To write them again, delete them and start the game: it writes them again by itself.
   `XIII.exe --mods-catalog <folder>` (`oxiii-view` when built from the sources) writes them into another
   folder at once.

In the game, two console commands (press **F2**, type, Enter) give names on the spot:

```text
modwho     names the character you look at, with a block to paste
modpos     gives the position and the facing where you stand
```

## The lists

| List | Where | What you paste |
|---|---|---|
| Characters, voices | [names/characters.md](names/characters.md) | `look = "Jones"` |
| Weapons | [names/weapons.md](names/weapons.md) | `weapon = "Pistol"` |
| Fixed words (team, behaviour, key names…) | [names/words.md](names/words.md) | `team = "enemy"` |
| Maps, checkpoints, characters of each map | `Mods/_Guide/names/maps.md` | `who = "gardien_lostpass"` |
| Objects and pickups, decorations | `Mods/_Guide/names/objects.md`, `decorations.md` | `class = "XIII.MedPick"` |
| Sounds, ambient sounds, music | `Mods/_Guide/names/sounds.md`, `ambient-sounds.md`, `music.md` | `"Package.Group.Name"` |
| Dialogue lines, events | `Mods/_Guide/names/dialogues.md`, `events.md` | `line = "Plage01_Pam_00"` |

The lists on your PC are made from your own copy of XIII, in your game's language; they hold
names only (no dialogue text).

## Reference pages

For the full property tables (every property of every class, with its type and the game's
value), `oxiii mods-reference <game folder> <output folder>` (oxiii built with
`--features mods`) writes `weapons.md`, `characters.md`, `character-models.md`, `animations.md`,
`maps.md`, `dialogues.md`, `sounds.md` and `content.md`.

## If it does not work

- No catalog in `Mods/_Guide`: the game writes it only in the improved build on a computer, a
  little while after it starts; `XIII.exe --mods-catalog <folder>` (`oxiii-view` when built from the
  sources) writes it at once.
- A name from a list is refused: copy it again; names are not case-sensitive but must be
  complete. The warning in `mods-log.txt` gives the closest names.

All the details: [expert/loading-and-testing.md](expert/loading-and-testing.md).
