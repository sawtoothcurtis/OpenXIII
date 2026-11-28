-- A first script: a short scene on the beach (Plage01).
--
-- The .lua files of a mod's scripts folder are run, in the order of their names, each time a
-- map starts (a checkpoint loads the map again, so they start again too). The code outside the
-- functions runs once, when the map starts: here it only says what to do and when.
--
-- Scripts run in the mods build only, on the game that has authority (solo, split screen, the
-- host of a network game). Errors are written to Mods/mods-log.txt with the file and the line;
-- they never stop the game. After editing this file, the console command modreload (F2) starts
-- the scripts of the map again, without leaving the game.

-- Only on the beach: on any other map, the script stops here.
if map() ~= "Plage01" then return end

-- Creates a [character] of this mod a few steps from the player (a spot can be in a wall:
-- a few tries around).
local function near_player(p, who, steps)
  local at = p.position
  for i = 1, 8 do
    local spot = {at[1] + steps * math.cos(i), at[2] + steps * math.sin(i), at[3]}
    local someone = spawn{character = who, position = spot}
    if someone then return someone end
  end
end

-- What to expect: play the beach (Brighton Beach) from its start. When the opening scene ends
-- (XIII has woken up in the hut; the map fires its first goal, Objectif91), General Carrington
-- appears a few steps away in the hut and walks up to XIII. When he is near, he says his line,
-- and a moment later a guard appears next to XIII. Mods/mods-log.txt follows each step.
on("event", "Objectif91", function(player)
  local visitor = near_player(player, "Visitor", 300)
  if visitor then
    visitor:go_to(player)              -- he walks up to XIII
  else
    log("no room for the visitor")
  end
end)

-- When XIII and he are within 200 units (find() knows him by his [character] name).
on("near", "Visitor", 200, function(player, visitor)
  if done then return end            -- one time only
  done = true
  visitor:look_at(player)
  say(visitor, "Hual02_Carrington_06")  -- a line of the game: "My best man, alive!"
  wait(4)                               -- seconds of game time; the game goes on meanwhile

  local guard = near_player(player, "Guard", 300)
  if guard == nil then
    log("no room for the guard")
    return
  end
  message("Someone was hiding behind the hut.")
  say(guard, "Don't move!")             -- your own subtitle (no voice)

  -- Ten seconds later, if he is still alive, he turns hostile.
  after(10, function()
    if guard.alive then
      guard.team = "enemy"
      log("the guard attacks, health " .. guard.health)
    end
  end)
end)

-- When he dies (a mod's [character] is found by its own name).
on("death", "Guard", function(guard)
  message("The guard is down.")
end)

on("start", function(player)
  log("first-script: the scene is ready on " .. map())
end)
