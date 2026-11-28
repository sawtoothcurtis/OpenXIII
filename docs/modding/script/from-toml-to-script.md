# From toml to script

Everything a `.toml` file does at the start of a map can also be done by a script, at the
moment you choose. Use the toml when "at the start of the level" is enough; use a script when
it depends on what happens.

## A character on the map

```toml
[character.Visitor]
base = "XIDCine.Cine2"
team = "friend"
properties = { Mesh = "XIIIPersos.CarringtonM", CodeMesh = 16, PawnName = "Carrington" }

[[map.Plage01.add]]
character = "Visitor"
position = [-470, -860, 1190]
yaw = 180
```

The same with a script (the `[character.Visitor]` block stays in the toml): he comes when the
opening scene of the beach ends.

```lua
if map() ~= "Plage01" then return end

on("event", "Objectif91", function(player)
  spawn{character = "Visitor", position = {-470, -860, 1190}, yaw = 180}
end)
```

## A line of dialogue

```toml
[[map.Plage01.say]]
by = "Carrington"
line = "Hual02_Carrington_06"
distance = 250
```

```lua
on("near", "Visitor", 250, function(player, visitor)
  say(visitor, "Hual02_Carrington_06")
end)
```

## Changing a character

```toml
[[map.Plage01.change]]
who = "Pam"
look = "Jones"
health = 300
```

```lua
on("start", function(player)
  change("Pam", {look = "Jones", health = 300})
end)
```

`change` takes the same words as the toml block (`look`, `skin`, `health`, `name`, `voice`,
`weapon`, `team`, `can_die`, `properties`) and finds the actor the same way.

## If it does not work

- A name the game does not know is written in `Mods/mods-log.txt`, as for the toml.
- `find` and `change` look for a name in the same order as the toml's `who`: the actor's name,
  then a character's Tag or PawnName, then another actor's Tag.
