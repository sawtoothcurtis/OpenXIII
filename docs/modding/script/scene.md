# Write a scene

A scene is a function that does things one after the other: a line, a wait, an order, the
camera. `wait` pauses the scene, never the game.

## Recipe

```lua
on("near", "Visitor", 250, function(player, visitor)
  visitor:look_at(player)
  say(visitor, "Hual02_Carrington_06")        -- a line of the game, with its voice
  wait(4)
  local at = visitor.position
  camera({at[1] - 150, at[2], at[3] + 70}, visitor, 3)  -- the camera, for 3 seconds
  wait(3)
  visitor:follow(player)                       -- from now on he follows XIII
end)
```

## Useful words

| Write | What happens |
|---|---|
| `say(who, "Plage01_Pam_03")` | a line of the game (catalog, Dialogues tab) |
| `say(who, "Hello.")` | your own subtitle |
| `wait(2)`, `wait_until(function() return ... end)` | waits two seconds, or until something is true |
| `who:go_to(position or actor)`, `who:follow(actor)`, `who:stop()` | walks there, follows, stops |
| `who:follow(player, {catch_up = 1500})` | follows; further than 1500 units and seen by no player, he is put back near the player |
| `who:look_at(...)`, `who:play("WaitNeutre", true)` | turns to look, plays an animation (looping) |
| `camera(position, look_at, seconds)`, `camera_back()` | every player's view, the cut-scene frame, controls frozen |
| `camera{from = p, look_at = who, path = {p2, p3}, seconds = 5}` | the camera goes along a path |
| `camera{from = p, look_at = who, orbit = {turns = 0.5}, seconds = 6}` | the camera turns around who |
| `camera{from = p, look_at = who, follow = who, fov = 60}` | the camera moves with who, field of view 60 |
| `focus{on = who, seconds = 3, zoom = true}` | the levels' "sixth sense" window on who (`danger = true`: the red one) |
| `slow_motion(0.4, 3)` | the game at 40 % speed for three seconds |
| `fade{to = "black", seconds = 2}`, `fade{back = true}` | the view fades to a colour, and back (`player = p`: that player's view only) |
| `who:cine("Anim Once Rire")` | for experts: a line of the levels' own scene language (story characters) |
| `message("text")`, `message("text", p)` | a message on every player's screen, or on player `p`'s only |
| `local id = sound("XIIISound.Ambient.SancFire", who)` then `stop_sound(id)` | a sound from who, stopped later; `music_stop()` stops the music |

## Shortcuts on characters and things

```lua
on("start", function(p)
  p:give("Shotgun")
  if p:has("Shotgun") and p.ammo then p.ammo = 50 end   -- the ammunition of the gun in hand
  local a = p:aim()                               -- where the player looks
  if a.actor and a.actor:is_a("Pawn") then log("aiming at", a.actor.name) end
  if game().mode == "network" then message("Good luck to both of you.") end
end)
```

`who:is_a("Pawn")` (or a full class, `"XIII.Beretta"`), `who:take("Shotgun")` takes a weapon away,
`who:aim()` gives `{from =, direction =, hit =, actor =}`, `game()` gives `{difficulty =,
language =, mode =}` (`"solo"`, `"split"` or `"network"`), and `preload("Package.Name")`, in the
script's main code, asks for an object to be made ready before the level starts.

## Comic-book insets

The levels show small comic-book windows over the game, each with its own camera: a guard
who heard something, a door that opens, XIII seen from above. `panel` shows them the same way,
on every player's screen (split screen and network game included).

```lua
on("event", "Objectif91", function(player)
  local at = player.position
  panel{
    {from = {at[1] + 150, at[2] + 60, at[3] + 60}, look_at = player, see_player = true,
     seconds = 4, x = 40, y = 40, text = "XIII?", appear = "left", sound = "tip", after = 1},
    {from = {at[1] - 150, at[2] - 60, at[3] + 120}, look_at = player, see_player = true,
     seconds = 3, x = 200, y = 40, word = "plage01a", appear = "zoom"},
  }
end)
```

| Setting | What it does |
|---|---|
| `from`, `look_at` | where the inset's camera stands, and what it looks at (an actor is followed) |
| `seconds`, `after` | how long the inset stays; seconds before the next inset of the list |
| `x`, `y`, `size` | place and size on a 640x480 screen; `size` up to 256, or `{width, height}` |
| `fov` | the camera's field of view (45 by default) |
| `text`, `word` | a caption, an onomatopoeia picture of the levels (`plage01a`, from XIIIvignettes) |
| `appear`, `sound` | `left`, `zoom`, `above`...; `tip`, `warning`, `dialog` |
| `see_player` | XIII himself is seen in the inset (hidden by default, as in the levels) |

Insets shown at the same time share one picture: four of the size 128 fit, as in the levels.

A position is `{x, y, z}`, a point on the ground: stand there in the game, open the console
(F2) and type `modpos`.

## If it does not work

- Characters find their way with the level's paths, as the game's own soldiers do, and push
  the doors on their way. Where the paths do not lead (a door the level keeps locked, a pier
  above the sand), they wait and the log says so: `catch_up` brings a follower back.
- A character of the map that still waits for its own scene does not take orders yet.
- A camera point inside a wall gives an error in the log rather than a black screen.
