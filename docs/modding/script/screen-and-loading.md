# The screen and the loading screen

A script can keep things on the screen: a text, a counter, a timer, a bar, a picture or a line.
They stay until the script removes them, and a new map starts without them. Between two maps, a
script can also draw its own loading screen and keep the next level waiting until a key is
pressed: the example mod `xbox-recaps` brings back the comic bubbles of the Xbox version this
way.

## Things on the screen

```lua
on("start", function(p)
  hud_text{id = "goal", text = "Find the radio", x = 0.5, y = 0.1, align = "centre", colour = "yellow"}
  hud_counter{id = "kills", label = "Enemies: ", value = 0, x = 0.05, y = 0.9}
  hud_timer{id = "bomb", label = "Bomb: ", seconds = 90, x = 0.85, y = 0.05}
  hud_bar{id = "boss", x = 0.3, y = 0.95, w = 0.4, h = 0.02, value = 1, colour = "red", back = {40, 40, 40}}
end)

local kills = 0
on("death", "Guard", function()
  kills = kills + 1
  hud_update("kills", {value = kills})
end)

after(90, function() hud_remove("bomb") end)
```

- `x` and `y` go from 0 to 1: `{0, 0}` is the top left corner of the view, `{1, 1}` the bottom
  right one. In split screen each player's view has its own; `player = p` puts an item on that
  player's view only.
- `size = "small"` or `"medium"` chooses the game's font (`font = "XIIIFonts.PoliceF20"` names
  one), `align` says which point of the text `x` is: `"left"`, `"centre"` or `"right"`.
- A colour is a name (`"white"`, `"black"`, `"grey"`, `"red"`, `"green"`, `"yellow"`, `"blue"`) or
  `{r, g, b}` from 0 to 255, with a fourth number for how solid it is.
- Each function gives the item's id (yours, or one it makes); an id already on the screen is
  replaced. `hud_update(id, {...})` changes some settings, `hud_remove(id)` takes one item away,
  `hud_remove()` all of them.
- `hud_image{texture = "Package.Texture", x =, y =, w =, h =}` stretches a texture of the game or
  of the mod over that part of the view; `hud_line{from = {x, y}, to = {x, y}}` draws a line.
- `text_size(text)` gives the width and the height of a line of text as parts of a 640 × 480
  screen: multiply by 640 and 480 for its pixels there.

In a network game, the host's items are shown on the guests' screens too (an item given
`player =` on that player's only).

## The loading screen

When the map changes, the scripts of the map being left hear it first:

| Event | The function receives | When |
|---|---|---|
| `"map_end"` | map | the map is left |
| `"loading"` | from_map, to_map, failed | the next map starts loading (`failed` is `"failed"` after a lost mission) |
| `"loaded"` | map | the next map is ready, while the start is held |
| `"loading_key"` | | a key, a mouse button, a gamepad button or the screen is pressed, while the start is held |

What a `"loading"` handler draws with the `hud_...` functions is the loading screen: a black
picture of 640 × 480 that the game scales to the window. It is shown only if the handler calls
`hold_start()`: the next level then loads behind it and does not start until a script calls
`release_start()`. Without `hold_start()` the game shows its usual loading screen.

```lua
on("loading", function(from, to)
  hud_text{text = "Next: " .. to, x = 0.5, y = 0.45, align = "centre", size = "medium"}
  hold_start()
end)

on("loaded", function()
  hud_text{text = "Press a key", x = 0.5, y = 0.9, align = "centre"}
end)

on("loading_key", function()
  release_start()
end)
```

- `"loaded"` and `"loading_key"` are run by the scripts of the new map (they start while it is
  prepared): write them in a script that runs on every map, as above.
- A press before the level is ready can release the start too: the game then shows its usual
  loading screen and starts the level by itself once it is ready.
- `text(section, key)` reads the game's own texts in the player's language: the Xbox recaps are
  `text("EndMsgMap", map)`. `game().language` says which language that is.
- The loading screen is not used for the co-op character screen, nor in a network co-op game (a
  player would wait for a key while the other one already plays).
- `wait()` does not wait in these handlers: nothing runs while the next map loads.

## The example: Xbox loading recaps

`examples/mods/xbox-recaps/scripts/recaps.lua` reads the recap of the map being loaded, splits it
into bubbles (`@`) and lines (`|`), places up to three bubbles as the console did, draws their
outlines with `hud_line` and their lines with `hud_text`, then holds the start. Once the level is
ready it adds "Press to continue" (in the player's language) and a press starts the level. To
change it: the places of the bubbles are `CENTRES`, the outline's colour `OUTLINE`, the prompt's
texts `PROMPT`.
