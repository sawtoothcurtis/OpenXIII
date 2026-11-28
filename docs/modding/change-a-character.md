# Change a character

Give a character the map already places another look, more health, a weapon or another team,
while it keeps its place and its role in the level.

## Recipe

```toml
[[map.PRock03.change]]
who = "gardien_lostpass"   # the character: Maps tab of the catalog, or modwho in the game
look = "Jones"             # a name of the Characters tab
health = 300
```

Names you can use here: [names/characters.md](names/characters.md) (looks and voices),
[names/weapons.md](names/weapons.md) (weapons), and `Mods/_Guide/names/maps.md` on your PC (the
`who` of every character of every map).

In the game, look at a character, press **F2** (the console) and type `modwho`: the game
prints its `who` and an entry ready to paste. The catalog's Maps tab lists every character of
every map with the same `who`.

## Settings

| Setting | What it does | Example |
|---|---|---|
| `look`, `skin` | another model, another texture on it | `look = "Kim"` |
| `health` | its health | `health = 500` |
| `weapon` | the weapon it carries (Weapons tab) | `weapon = "Shotgun"` |
| `team` | `friend`, `neutral` or `enemy`: how it treats XIII | `team = "friend"` |
| `can_die` | `false`: it cannot be hurt or killed | `can_die = false` |
| `name`, `voice` | its in-game name, the voice of its shouts | `voice = "Carrington"` |

The changes are made when the level starts, before any of its scripts run, and again at each
load. `near = [x, y, z]` (and `radius`) picks one of several characters with the same name.
`behaviour` cannot be changed here: the character keeps its role; to have another kind of
character, [spawn one](spawn-a-character.md).

To change every character of a kind (all the soldiers, everywhere), use a class table instead:

```toml
[class."XIDPawn.BaseSoldier"]
Health = 200
apply_to_placed = true
```

## If it does not work

- `mods-log.txt` says "no actor named …": copy the name from `modwho` or the Maps tab; the log
  lists the closest names.
- A wrong look or weapon gives one warning with the closest names; the rest still applies.
- `weapon` is given when the character's AI starts: a character without AI (some cut-scene
  people) does not use it.
- In a network game the host makes the change and every player sees it: the model, and the
  `skin`, which OpenXIII sends to the guests itself (see [multiplayer-and-coop.md](multiplayer-and-coop.md)).

All the details: [expert/characters.md](expert/characters.md#changing-one-character-of-a-map).

The same at the moment you choose, with a script: [script/from-toml-to-script.md](script/from-toml-to-script.md).
