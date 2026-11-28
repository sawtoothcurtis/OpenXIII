# Objectives, doors and the end of a level

## Recipe

```lua
on("event", "Objectif91", function(player)
  objective("Find the boat")               -- shown as the game's own objectives
  trigger("PorteCabane")                   -- fires an event of the map: doors, lifts, scenes
end)

on("enter", {-209, -2559, 1135}, 300, function(player)  -- on the beach, below the hut
  objective_done("Find the boat")
  next_level("Banque01")                   -- the mission is won; on to that map
end)
```

## Useful words

| Write | What happens |
|---|---|
| `objective("text")`, `objective_done("text")` | a new objective, a reached one (`objective("text", p)`: for player `p` only) |
| `trigger("name")` | the map's event `name`: what the level designer bound to it happens (a door opens, a lift moves) |
| `find("Door")` then `door:trigger()` | triggers that actor itself (a door, a mover, an emitter), caused by the player |
| `hud_timer{id = "boat", label = "Boat: ", seconds = 60, x = 0.8, y = 0.05}` | a countdown on the screen ([the screen](screen-and-loading.md)) |
| `next_level("Map")` | the mission is won, the game goes to that map (with `#Teleporter` to arrive at a given place) |
| `checkpoint("On the beach")` | the levels' save point: the game saves here (it takes up again at the map's start, or at a teleporter of the map given second) |
| `fail("text")` | the mission is lost, with that text on the game over screen |
| `campaign{"MapA", "MapB"}` then `next_level()` | your own maps in order (a mod's `maps` folder can hold its own `.unr` maps) |

`save` is your mod's memory: a table that stays from one map to the next and comes back with a
checkpoint (numbers, strings, true/false and tables of them).

```lua
save.visits = (save.visits or 0) + 1
log("visit number " .. save.visits)
```

## If it does not work

- The names of the map's events are in the catalog (`Mods/_Guide/names/events.md`).
- `next_level` works on the maps of the story (and your maps made from them); elsewhere the
  level travels without the "mission complete" screen.
- In a network game, objectives, sounds and music reach every player; a scene's camera is
  shown on this computer's screens only.
