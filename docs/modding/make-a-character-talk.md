# Make a character talk

Have any character say a line of the game (with its real voice and its subtitle in the
player's language) or a line of your own, when the level starts, when XIII comes near, or
when something happens.

## Recipe

```toml
[[map.Plage01.add]]
character = "Carrington"           # he stands in the beach hut
position = [-470, -860, 1190]

[[map.Plage01.say]]
by = "Carrington"                  # who speaks: a character of the mod or of the map
line = "Hual02_Carrington_06"      # a line of the game: Dialogues tab of the catalog
distance = 250                     # when XIII comes this near
```

The line comes from your own copy of the game: its voice from the level's voice package, its
subtitle in the language of your game. Any character can say any line, on any map.

Names you can use here: on your PC, `Mods/_Guide/names/dialogues.md` (line names and speakers),
`Mods/_Guide/names/events.md` (for `when = "event"`) and `Mods/_Guide/names/maps.md` (who is on each
map); [names/characters.md](names/characters.md) (characters to add).

## Settings

| Setting | What it does | Example |
|---|---|---|
| `by` | who speaks: a character the mod adds (its name), or a character of the map (its name in the game, the `who` of the Maps tab) | `by = "Pam"` |
| `line` | a line of the game | `line = "Plage01_Pam_03"` |
| `text`, `sound` | your own subtitle, and a sound (yours or the game's) | `text = "Hello."` |
| `when` | `near` (the default, with `distance`), `start` (the level starts), `event` (with `event = "<name>"`) | `when = "start"` |
| `delay` | seconds to wait first | `delay = 1` |

A line of your own, with your recording:

```toml
[sounds.MyLine]
file = "sounds/my_line.ogg"

[[map.Plage01.say]]
by = "Pam"
text = "My own line."
sound = "Mod.MyLine"
when = "start"
```

## If it does not work

- Nobody speaks: `mods-log.txt` says when `by` finds nobody on the map (the closest names are
  listed) or when a line name is unknown.
- `when = "near"` waits until the opening scene is over: XIII does not walk during the game's
  cut-scenes.
- Lines triggered together are said one after the other, never over the game's own dialogues.
- A line without a voice in the game (XIII's thoughts) shows its subtitle only.

All the details (events, voices of other levels, subtitles): [expert/dialogues.md](expert/dialogues.md).

The same at the moment you choose, with a script: [script/from-toml-to-script.md](script/from-toml-to-script.md).
