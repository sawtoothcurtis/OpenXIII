# Build a mission

A complete mission on the beach: after the opening scene, hold the beach against three waves
of soldiers.

## The mod

`mod.toml` says what the soldiers are:

```toml
[mod]
name = "Beach mission"
version = "1.0.0"

[character.Raider]
base = "XIDPawn.BaseSoldier"
team = "enemy"
properties = { Mesh = "XIIIPersos.SpadsM" }
```

`scripts/mission.lua` says what happens:

```lua
if map() ~= "Plage01" then return end

local function wave(number, player)
  message("Wave " .. number .. " of 3")
  local raiders = {}
  for i = 1, number + 1 do
    local r = spawn{character = "Raider", near = player}   -- a spot with room and ground
    if r then raiders[#raiders + 1] = r r:attack(player) end
  end
  wait_until(function()
    for _, r in ipairs(raiders) do if r.alive then return false end end
    return true
  end)
end

on("event", "Objectif91", function(player)            -- the opening scene is over
  save.tries = (save.tries or 0) + 1
  objective("Hold the beach")
  for number = 1, 3 do wave(number, player) wait(5) end
  objective_done("Hold the beach")
  message("The beach is yours (try " .. save.tries .. ").")
end)
```

## How it works

- The mission starts with an event of the map: `Objectif91` is fired when the opening scene
  ends (the catalog lists the events of each map).
- `spawn{..., near = player}` takes the first spot around the player with room and ground; a
  spot given with `position` in a wall or above a drop is refused (the log says why).
- `wait_until` waits until every soldier of the wave is down; the game goes on meanwhile.
- `save.tries` counts the tries: it comes back with a checkpoint and goes on to the next maps.

## If it does not work

- Soldiers who stay where they are: `attack` makes them come for the player; their AI may take
  a moment to set up.
- To end with a new map: `next_level("Banque01")` after `objective_done`.
