# React to something

`on(event, ..., function ... end)` runs the function each time the event happens.

## Recipe

```lua
on("near", "Visitor", 250, function(player, visitor)
  message("Someone is waiting for you.")
end)

on("death", "Guard", function(guard)
  message("The guard is down.")
end)

on("key", "K", function(player)
  player.health = 100
end)
```

## The events

| Event | Then | The function receives | When |
|---|---|---|---|
| `"start"` | | player | the map has started |
| `"near"` | who, distance | player, actor | a player comes near who (not during a cut-scene) |
| `"enter"` | position, radius | player | a player comes into a zone |
| `"sees"` | who | actor, player | who sees a player, with its own eyes |
| `"death"` | who | actor | who dies |
| `"damage"` | who | actor, amount, by, damage_type, weapon | who loses health: by is who hit (nil when the game does not say), damage_type the kind of damage, weapon the class of the weapon by held |
| `"fire"` | [who] | shooter, weapon_class | each shot of a player (no who), of who, or of every character (`"all"`) |
| `"pickup"` | | player, class | an item comes into a player's inventory |
| `"event"` | name | player | the map fires that event (a trigger, a door, a scene) |
| `"key"` | key | player | a player presses that key |
| `"map_end"`, `"loading"`, `"loaded"`, `"loading_key"` | | | the change of map: see [the screen and the loading screen](screen-and-loading.md) |

Who fired, who hit, and with what:

```lua
on("fire", function(shooter, weapon)
  if weapon == "XIII.Beretta" then log(shooter.name, "fired the pistol") end
end)

on("damage", "Guard", function(guard, amount, by, kind, weapon)
  if by == player() and kind == "XIII.DTGunned" then message("Good shot.") end
end)
```

`after(seconds, fn)` runs a function later, `every(seconds, fn)` again and again; `on`, `after`
and `every` give a number that `cancel(number)` takes to stop them.

```lua
local ticking = every(1, function() log("one more second") end)
after(5, function() cancel(ticking) end)
```

To start once the opening scene is over and XIII can move, whatever the map:

```lua
on("start", function(player)
  wait(1)                    -- the first frames come before the opening scene
  wait_until(has_control)    -- no cut-scene is playing
  message("Your turn.")
end)
```

## If it does not work

- `who` is found as in the toml (`find` gives the actor or nil): try `log(tostring(find("Visitor")))`.
- A character of the map may be there only for a while: on Plage01, Janet (Pam) is only there
  during the opening scene, so `on("near", "Pam", ...)` never fires once XIII can move.
- The game window keeps F3 to F9 and F11 (co-op character screen, cheats of the
  test builds): a `"key"` event on one of them never runs, and the log says so. Free keys: K, L,
  J, H, U, O, P, Insert, NumPad2 to NumPad9.
- The names of the map's events are in the catalog (`Mods/_Guide/names/events.md`).
- A function that fails writes its file and line in `Mods/mods-log.txt`; the others go on.
