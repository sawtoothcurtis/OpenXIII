-- A small mission on the beach (Plage01): three waves of soldiers to hold off.
--
-- What it shows: an objective on the screen, soldiers created in waves around the player,
-- waiting until a wave is down, and a counter kept in `save` (it stays when a checkpoint is
-- loaded, and goes on to the next maps of the campaign).

if map() ~= "Plage01" then return end

local WAVES = 3

-- Creates one soldier somewhere around the player (a few tries: a spot may be in a wall).
local function raider_near(p)
  local at = p.position
  for _ = 1, 6 do
    local angle = random() * 2 * math.pi
    local distance = random(500, 900)
    local spot = {at[1] + distance * math.cos(angle), at[2] + distance * math.sin(angle), at[3]}
    local r = spawn{character = "Raider", position = spot}
    if r then return r end
  end
  return nil
end

-- One wave: n soldiers, then wait until they are all down.
local function wave(number, p)
  message("Wave " .. number .. " of " .. WAVES)
  local raiders = {}
  for i = 1, number + 1 do
    local r = raider_near(p)
    if r then
      raiders[#raiders + 1] = r
      r:attack(p)
    end
  end
  wait_until(function()
    for _, r in ipairs(raiders) do
      if r.alive then return false end
    end
    return true
  end)
end

-- The mission starts when the opening scene ends (the map fires its first goal, Objectif91).
on("event", "Objectif91", function(p)
  save.tries = (save.tries or 0) + 1
  objective("Hold the beach")
  wait(3)
  for number = 1, WAVES do
    wave(number, p)
    save.best = math.max(save.best or 0, number)
    wait(5)
  end
  objective_done("Hold the beach")
  message("The beach is yours (try " .. save.tries .. ").")
end)
