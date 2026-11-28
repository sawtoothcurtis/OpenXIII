-- Guard dogs: where they are.
--
-- One block per level. Each dog is placed next to a character of the level (its master,
-- found by the name the catalog gives him: Mods/_Guide/names/maps.md); a master who is not in
-- the level any more (a checkpoint after his death) has no dog.
--
-- A dog comes next to its master; where the place matters, `at` gives the point (taken in
-- the game with the console command `modpos`).

local here = map()

-- The dogs come once the level's opening scene is over.
on("start", function()
  wait(1)
  wait_until(has_control)

  if here == "Toits01" then
    -- The rooftops. The game's own files call the man who kicks the table over
    -- "the dog's owner" (his death fires the event MortProprioChien): here is his dog at
    -- last. It waits behind the table with him, and when XIII comes through the door it runs
    -- round the table, on XIII's left, and straight at him.
    local dog = Dog.place{
      master = "SurpriseTempo", ground = "tiles", leash = true,
      at = {1450, 3650, 0}, yaw = 180,
      flank = {{1290, 3470, 0}},
    }
    -- The level fires SurpriseTempo when the door opens: the man kicks the table, and
    -- lets go of the dog.
    on("event", "SurpriseTempo", function(player) Dog.sic(dog, player) end)

  elseif here == "Hual04a" then
    -- The cable car station, in the snow: two guards walk their rounds with a dog, the one
    -- far out on the left and the first one of the station.
    Dog.place{master = "BaseSoldier1", ground = "snow"}
    Dog.place{master = "BaseSoldier5", ground = "snow"}

  elseif here == "Kello01a" then
    -- Kellownee Lake. The two men who split up after the first talk each take a dog...
    Dog.place{master = "BaseSoldier0", ground = "snow"}
    Dog.place{master = "BaseSoldier9", ground = "snow"}
    -- ...and so does the man alone after the checkpoint, who calls for the others.
    Dog.place{master = "FourthSpeech", ground = "snow"}
    -- One more by the lake, with the men who wait there.
    Dog.place{master = "nearest", ground = "snow", at = {-4547, -5399, -1296}, yaw = 249}

  elseif here == "Amos01" then
    -- The FBI building, upstairs: the guard who walks there (by his name; started from a
    -- checkpoint, where names change, whoever is nearest).
    local at = {-1081, -831, 417}
    local _ = Dog.place{master = "BaseSoldier9", at = at, yaw = 156}
      or Dog.place{master = "nearest", at = at, yaw = 156}

  elseif here == "Hual01a" then
    -- Emerald Base, the bridge. The two guards who walk from the trucks towards XIII are
    -- not there when the level starts: the level makes them when it fires
    -- GuardsArrivalthird. Their dog comes with them, next to the first of the two.
    on("event", "GuardsArrivalthird", function()
      wait(0.6)
      local master = Dog.nearest_master({1930, -1570, -390}, 600)
      if master then Dog.place{master = master} end
    end)

  elseif here == "Hual01b" then
    -- Emerald Base, the roof.
    Dog.place{master = "nearest", at = {3005, -1552, -1146}, yaw = 15}

  elseif here == "Kello01b" then
    -- The hideout at Kellownee.
    Dog.place{master = "nearest", at = {-998, -1874, 1989}, yaw = 154}

  elseif here == "PRock04a" then
    -- The canyon, first part: two guards.
    Dog.place{master = "nearest", ground = "gravel", at = {12485, -2831, 1258}, yaw = 272}
    Dog.place{master = "nearest", ground = "gravel", at = {11923, -3554, 1258}, yaw = 272}

  elseif here == "PRock04b" then
    -- The canyon, second part: near the start, on the way down, and after the second
    -- checkpoint ("Canyon 6") with one of the two men at the very bottom.
    Dog.place{master = "nearest", ground = "gravel", at = {18055, -1710, 46}, yaw = 356}
    Dog.place{master = "nearest", ground = "gravel", at = {12251, 4961, 372}, yaw = 177}
    Dog.place{master = "BaseSoldier9", ground = "gravel"}

  elseif here == "Sanc01" then
    -- The Sanctuary's garden: a dog at ease, lying by the fountain while its master walks
    -- his round; another with a guard further on.
    Dog.place{master = "nearest", lying = true, at = {-3430, 10644, 1}, yaw = 272}
    Dog.place{master = "nearest", at = {-2578, 6497, 273}, yaw = 159}
  end
end)
