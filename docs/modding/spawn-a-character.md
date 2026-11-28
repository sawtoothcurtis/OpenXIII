# Spawn a character

Put a character of the game on a map, or make one appear in front of the player when you
press a key (a keyboard shortcut).

## Recipe

```toml
[[map.Plage01.add]]
character = "Jones"              # a name of the catalog's Characters tab
position = [-470, -860, 1190]    # a point on the ground (console command modpos)
yaw = 180                        # the way it faces, in degrees (optional)
```

That is all: Major Jones stands in the beach hut, calm, a friend of XIII. With a keyboard
shortcut instead, the character appears in front of the player when you press K:

```toml
[[shortcut]]
key = "K"
spawn = "Carrington"
behaviour = "fights"
weapon = "Shotgun"
```

Names you can use here: [names/characters.md](names/characters.md) (characters and voices),
[names/weapons.md](names/weapons.md) (weapons), [names/words.md](names/words.md) (team, behaviour),
and `Mods/_Guide/names/maps.md` on your PC (positions are given by `modpos`).

## Settings

Add any of these settings to the block (`[[map.<Map>.add]]` or `[[shortcut]]`):

| Setting | What it does | Example |
|---|---|---|
| `behaviour` | `stands` (calm, stays there), `patrols` (a soldier with the game's AI), `fights` (a soldier who attacks XIII) | `behaviour = "fights"` |
| `weapon` | the weapon it carries (Weapons tab) | `weapon = "Pistol"` |
| `team` | `friend`, `neutral` or `enemy`: how it treats XIII; `enemy` alone makes a soldier that fights (as `behaviour = "fights"`) | `team = "enemy"` |
| `health`, `can_die` | its health; `can_die = false` makes it unkillable | `health = 300` |
| `look`, `skin` | another model, or another texture on it | `look = "Kim"` |
| `name`, `voice` | its in-game name (for dialogues) and the voice of its shouts | `voice = "Carrington"` |

`stands` is the default. A standing character is a story character of the game (the people of
the cut-scenes): it never attacks and stays where it is, whatever its team. `patrols` and `fights` make a soldier
(the game's soldier AI); a soldier created by a mod has no patrol path of its own (the map's
paths belong to the map's soldiers). A soldier attacks XIII once it sees or hears him; one made by
a reaction's `spawn` with `team = "enemy"` comes for XIII at once ([reactions.md](reactions.md)).
`position` is where its feet are; `from_checkpoint = 2`
adds it only when the level is started from that checkpoint.

To reuse the same character in several places, define it once:

```toml
[character.Bodyguard]
look = "Spads"
behaviour = "fights"
weapon = "AssaultRifle"
```

then write `character = "Bodyguard"` or `spawn = "Bodyguard"`. Settings written in a block go over
the ones of its `[character]`; raw `properties = { … }` in the entry win over everything.

## If it does not work

- Open `Mods/mods-log.txt`. A wrong name gives one warning with the closest names
  ("did you mean Jones?"); a position outside the map says so.
- The character falls or does not appear: take the position again with `modpos`, standing
  where it should stand.
- It looks like XIII: its class has no look of its own; give it a `look`.
- In a network game the server creates the characters; a keyboard shortcut only changes the game
  of the player who pressed it.

All the details (AI states, death animations, raw values): [expert/characters.md](expert/characters.md).

The same at the moment you choose, with a script: [script/from-toml-to-script.md](script/from-toml-to-script.md).
