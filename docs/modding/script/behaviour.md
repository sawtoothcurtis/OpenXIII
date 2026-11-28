# Invent a behaviour

A behaviour is a function that drives a character for as long as he lives: a guard who walks
his round, a friend who follows, someone who runs away.

## Recipe

```lua
behaviour("Watchman", function(self)
  self:patrol{self.position, player().position}        -- between his place and the player's
  wait_until(function() return self.health < 100 end)  -- until he is hurt
  self:stop()
  message(self.name .. " calls for help!")
end)

on("start", function(player)
  spawn{character = "Visitor", near = player, behaviour = "Watchman"}
end)
```

## Useful words

| Write | What happens |
|---|---|
| `behaviour("Name", function(self) ... end)` | defines a behaviour (`self` is the character) |
| `spawn{..., behaviour = "Name"}`, `who:behave("Name")` | gives it to a character; `who:behave(nil)` stops it |
| `who:patrol{point, point, ...}` | walks from point to point, again and again (a soldier only between points it can walk straight to) |
| `who:attack(actor)` | a soldier takes this actor as its enemy |
| `characters_near(position or actor, radius)` | the characters around, nearest first |
| `on("sees", who, fn)` | who sees a player, with its own eyes |
| `on("damage", who, fn(actor, amount, by))` | who is hurt, and by whom: `self:attack(by)` hits back |
| `on("fire", "all", fn(shooter, weapon))` | someone fired: a guard who hears a shot can come and look |

A guard who answers a shot by coming to look, and hits back at whoever hurts him:

```lua
behaviour("Listener", function(self)
  on("fire", function(shooter) if self.alive then pcall(self.go_to, self, shooter.position) end end)
  on("damage", self, function(me, amount, by) if by and by ~= me then me:attack(by) end end)
  while self.alive do wait(1) end
end)

on("start", function(player)
  spawn{character = "Guard", near = player, behaviour = "Listener"}
end)
```

For the experts: `who:get("Property")`, `who:set("Property", value)` and
`who:call("Function", ...)` reach any property or function of the game's classes (the
catalog lists them).

```lua
on("start", function(player)
  log("XIII runs at " .. player:get("GroundSpeed"))
end)
```

## If it does not work

- A behaviour stops when its character dies; a mistake in it is written in the log.
- Points must be on the ground: `spawn` refuses a point above a drop (he would fall and be
  hurt), and `spawn{..., near = player}` finds a spot with room and ground by itself. A patrol
  between points you know (where someone stands, `modpos`) never sends him over an edge.
- Soldiers keep their own AI: they fight, hear and run as soldiers do; orders steer them.
- A wrong property name or type gives a line in the log with the expected type.
