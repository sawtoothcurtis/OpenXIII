# Multiplayer and co-op

Play a game with mods with other players on the network: everyone must have the same mods,
and the host makes the changes of the maps.

## Recipe

1. Make the mods work alone first.
2. Copy the host's whole Mods folder to every player (same folders, same files).
3. Host the game as usual. Players with other mods, or without mods, are refused with a
   message that says why.

```text
Your mods differ from the server's (blaster 1.0.0): mod blaster: version 1.0.1 instead of 1.0.0
```

## What is shared

| Change | In a network game |
|---|---|
| Class values (`[class."…"]`), new classes, own sounds, textures, models | the same on every side: each side reads the same mods |
| Characters and objects added to maps, `remove`, `replace`, start weapons | made by the host; the players receive the actors as the game sends them |
| `change` of a placed character | made by the host; every player sees the new look, size and skin |
| `skin` of a character or object (`change`, `add`, `spawn`, `[character]`) | the game never sends skins; OpenXIII sends the skins a mod chose to each guest, so host and guests see the same |
| `team`, `can_die`, `behaviour`, `weapon` | used by the host's AI; the players see what the characters do |
| Scripts (`scripts/*.lua`) | run by the host only; the screen items of `hud_text`, `hud_bar`... reach every guest (`player =`: that guest only); `message`, `objective` and `fade` given a player go to that player only, on his own computer; `on("fire")` hears a guest's shots on the host (his gun is fired there); a script's loading screen (`hold_start`) is not used in a network game; what they show (`effect{}` actors and what is done to them, `charred`, `char_area`, `retexture`, `heat`, sounds) is sent to every guest, also to one who joins late: see "In a network game" in [script/functions.md](script/functions.md) |
| Keyboard shortcuts (`[[shortcut]]`) | only change the game of the player who pressed the key |
| Split screen (two players on one PC) | one game: both views always show the same thing |

Every side writes the mods' identities in its log when it starts, for example
`mods: network identity of mod blaster 1.0.0 (3F1C0D2E9A7B5C41)`: the codes must be the same
everywhere. Changing any file of a mod changes its code.

## If it does not work

- "Rejected by server": compare the identities in both logs and copy the host's Mods folder
  again.
- A player with mods can join a host without mods: its mods then only change its own game.
- The faithful build and the original game cannot join a host with mods.

All the details: [expert/content-and-network.md](expert/content-and-network.md#mods-in-network-games).
