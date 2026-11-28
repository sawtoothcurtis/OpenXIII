# Keyboard shortcuts

Make a key of the keyboard do something during play: press K and a character appears, press
F10 and XIII gets a weapon, or a sound plays, or a console command runs.

## Recipe

```toml
[[shortcut]]
key = "K"                    # the key you press
spawn = "Jones"              # a character appears in front of the player

[[shortcut]]
key = "F10"
give = "XIII.Bazook"         # a weapon for the player (its class)

[[shortcut]]
key = "J"
command = "modpos"           # a console command, as if typed
```

Names you can use here: [names/characters.md](names/characters.md) (characters),
[names/weapons.md](names/weapons.md) (weapons), [names/words.md](names/words.md) (key names).

## Settings

Each `[[shortcut]]` block has these settings:

| Setting | What it does | Example |
|---|---|---|
| `key` | the key you press: F1…F12, A…Z, 0…9, NumPad0…NumPad9, Insert, Delete, Home, End, PageUp, PageDown | `key = "K"` |
| `spawn` | a character (a name of the characters list, or a `[character]` of the mods), with the character settings of [spawn-a-character.md](spawn-a-character.md) | `spawn = "Kim"` |
| `in_front_of_player` | how far in front it appears (150 by default; where there is no room, at the first spot around with room) | `in_front_of_player = 250` |
| `give` | a weapon for the player (its class, Weapons list) | `give = "XIII.m16"` |
| `sound` | a sound played at the player | `sound = "Mod.MyHum"` |
| `command` | a console command | `command = "modwho"` |

One shortcut does one thing: write exactly one of `spawn`, `give`, `sound` or `command`. The cheat
build of the game already uses F5, F6, F7 and F11: choose other keys. In the console,
`modkey K` does what the K shortcut does without pressing it.

Older mods write `[[action]]` instead of `[[shortcut]]`: both work the same.

## If it does not work

- The key does nothing: `mods-log.txt` lists every keyboard shortcut ("keyboard shortcut K")
  and says when a name is wrong. The game window keeps F3, F4, F8 and F9 (and F5, F6, F7, F11 in
  the test builds): a shortcut on them never runs, and the log says so when the game starts.
  Free keys: K, L, J, H, U, O, P, Insert, NumPad2 to NumPad9.
- Two mods use the same key: the mod whose folder comes last wins; the Mods page shows the
  conflict.
- In a network game a shortcut only changes the game of the player who pressed the key.

All the details (characters' AI states, looks per state): [expert/characters.md](expert/characters.md#keys-and-character-behaviour).
