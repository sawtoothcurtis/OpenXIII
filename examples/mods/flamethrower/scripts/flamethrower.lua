-- The flamethrower: its flames, the characters it sets on fire, the marks it leaves.
-- Every duration is counted with the game's clock (time()), never in frames: the flame burns
-- the same at 30, 60 or 240 frames per second.

Flamethrower = {
  -- The weapon is given to every player at the start of each level (and to a network guest when
  -- he arrives), put in hand, its fuel filled up to `start_fuel` (false: give it another way: a
  -- pickup or a start weapon in a mod.toml).
  give_at_start = true,
  start_fuel = 300,
  -- The key that fills the fuel up while the flamethrower is in hand (false: none).
  refill_key = "L",
  -- Effects that can be turned off (false).
  fire_clouds = true,       -- the Sanctuary's great soft clouds of fire over a blaze in full fury
  sparks = true,            -- sparks and small flamelets (the Sanctuary's) flying up from blazes
  -- How many fine sparks a blaze throws (1: as the Sanctuary's fire; 2: twice as many...): the
  -- sparks raining from what burns above, and those rising above the roof and drifting away.
  -- They grow with the fire: a third of them while it spreads, all in its fury.
  particle_amount = 1.0,
  -- The sound of a blaze, in layers played from its middle, each with its own reach (game units;
  -- a man is about 170 tall, so about 94 a metre): full up to `near`, then fading smoothly to
  -- `far_volume` decibels at `far`, not heard beyond. `stages`: when it plays (1: the fire takes
  -- hold, 2: its fury, 3: it calms down). `loud`: how many times it is played at once (2:
  -- louder). The near layers are heard close by, the far one from up to about 95 m away.
  blaze_layers = {
    {name = "close",      sound = "XIIISound.Ambient.Sanc02bFire2",    near = 300,  far = 3000, far_volume = -40, stages = {1, 3}, loud = 1},
    {name = "close fury", sound = "XIIISound.Ambient.SancFireLoop",    near = 500,  far = 3500, far_volume = -40, stages = {2},    loud = 1},
    {name = "middle",     sound = "XIIISound.Ambient.Sanc02bFireLoop", near = 1500, far = 6000, far_volume = -40, stages = {1, 2, 3}, loud = 1},
    {name = "far",        sound = "XIIISound.Ambient.HelicoFireLoop",  near = 3500, far = 9000, far_volume = -40, stages = {2, 3}, loud = 1},
  },
  -- Played once when the blaze bursts into its fury.
  blaze_kick = {sound = "XIIISound.Ambient.Sanc02bFireStart", near = 1000, far = 7000, far_volume = -40},
  -- Crackles, played again and again from the blaze: by stage, then after it is out.
  blaze_crackles = {
    [1] = {sound = "XIIISound.Ambient.FireSpad2", near = 300, far = 2500, far_volume = -40},
    [2] = {sound = "XIIISound.Ambient.FireChimney1", near = 400, far = 3000, far_volume = -40},
    [3] = {sound = "XIIISound.Ambient.FireSpad2", near = 300, far = 2500, far_volume = -40},
    -- Once the fury has lasted `after` seconds, a softer second crackle with it.
    peak = {sound = "XIIISound.Ambient.FireSpad0", near = 150, far = 1500, far_volume = -40, after = 10},
    -- After the blaze, while its embers glow, for `seconds`.
    out = {sound = "XIIISound.Ambient.FireSpad1", near = 200, far = 2000, far_volume = -40, seconds = 20},
  },
  body_sparks = false,
  -- Smoke: thick black smoke over a blaze in its fury, smoke hanging under the ceilings of a
  -- burning building, and smoke still rising from burnt places after the fire is out (seconds).
  fury_smoke = true,
  ceiling_smoke = true,
  after_smoke_seconds = 150,
  -- The blast of a burning building: once `blast_share` of it burns or is burnt, the fire bursts
  -- out of its windows and doors on every side, breaking them and what can break around, with
  -- the game's grenade explosion (its flash, debris, smoke and its "BLAM"), its sound, and a push
  -- for whoever stands close; then the whole building is ablaze. Only for a blaze on wood.
  blast = true,
  blast_share = 0.75,       -- the part of the building that must be burning or burnt
  blast_building = 1000,    -- how far around the first blaze the building is looked for
  blast_reach_sample = 260, -- a place of the building burns once a flame is this close to it
  blast_min_seconds = 22,   -- and not before the building has burned that long, in its fury
  blast_breaks_all = true,  -- the blast also breaks what the level uses as a signal (a window)
  blast_reach = 520,        -- how far it hurts and pushes characters (and the players)
  blast_damage = 25,        -- health lost at its middle (less further away)
  -- Its sound: the Sanctuary's explosion, then a roof's a moment later: {sound, seconds after,
  -- decibels below the first}. The first is heard far away with `blast_sound_range`; the others
  -- follow it, that much softer for the nearest player. (Another explosion of the Sanctuary,
  -- louder: "XIIISound.Explo.ExSanc2".)
  blast_sounds = {{"XIIISound.Explo.ExSanc1", 0, 0}, {"XIIISound.Explo.ExToit1", 0.06, 8}},
  blast_sound_range = {near = 300, far = 9000, far_volume = -40},
  char_scenery = true,      -- everything a blaze burns (walls, roof, furniture, bodies) turns black for good
  embers = false,           -- embers glowing after a blaze
  wall_glow = false,        -- the light of the flames projected on the walls
  wood_spread = true,       -- wood burns faster and the fire creeps along it
  charred_wood = false,     -- charred planks left on burnt wood
  smoke_column = false,     -- a column of mixed smoke above a big blaze, with flying embers
  tall_flames = false,      -- the Sanctuary's tall flames on wood that burns
  weapon = "Mod.Flamethrower",
  range = 650,          -- how far the flame reaches (units; a man is about 170 tall)
  cone = 14,            -- half-angle of the flame, in degrees, plus `width` at the nozzle
  width = 30,
  flame_damage = 40,    -- health lost per second inside the flame
  burn_damage = 12,     -- health lost per second while burning: a character burns until he dies
  char_seconds = 7,     -- seconds of burning to turn a character (or his body) black
  corpse_seconds = 25,  -- a dead body keeps burning that long
  panic_chance = 0.7,   -- how many soldiers on fire run away in panic (the others fight on)
  prop_damage = 20,     -- damage per second given to an object in the flame (crates break, barrels blow up)
  -- The heat of what burns breaks the objects around that a bullet breaks (bottles, glasses,
  -- window panes, shutters), with the game's own damage: each breaks with its own effect and sound. The
  -- damage per second is the rate next to the fire, less and less further away, none beyond the
  -- reach. Objects whose breaking sets off something in the level (an Event) are left alone.
  heat_props = {fire_reach = 180, fire_rate = 6, blaze_reach = 520, blaze_rate = 14},
  -- The game's own fire sounds, played from the flames of what burns (a character, a fire on the
  -- scenery): they stop when those flames are gone. The first one the game can play is used.
  -- The jet: the gas of the submarine levels' steam pipes, looping while the fire button is held.
  jet_sounds = {"XIIISound.Ambient.SteamLoop3", "XIIIsound.Ambient.SteamLoop3"},
  jet_loudness = 2.4,   -- how loud the jet's sound is (1: as the game plays it, a soft hiss; 2:
                        -- twice as loud...; fractions allowed)
  -- Played once when the fire button is pressed: the M16 grenade launcher's shot and the thud of
  -- XIII hit by a grenade, with their volume (1: as in the game; 0.55: softer) and, if given,
  -- after how many seconds they are cut.
  burst_sounds = {
    {"XIIISound.Guns__M16Fire.M16Fire__hM16GrenadeFire", 0.55, 0.25},
    {"XIIISound.XIIIPerso.XIIIPafGren1", 1.2},
  },
  burst_gap = 0.35,     -- seconds: quicker presses do not play them again
  -- Fires left on the scenery hurt whoever stands in them: characters catch fire, the players
  -- lose health (the jet itself never hurts the one who holds it).
  ground_fire_reach = 70,     -- units around a fire (and around each of its tongues)
  ground_fire_damage = 12,    -- health a player loses per second in a fire
  -- A blaze: an area (cells of `blaze_cell` units) that has taken `blaze_heat` seconds of flame
  -- (the jet on it, fires burning in it) bursts into big flames over everything around, its
  -- roof included, burns `blaze_seconds`, then dies down to embers; meanwhile it heats the areas
  -- next to it, which may catch in turn.
  blaze_cell = 400,
  blaze_heat = 4,
  blaze_seconds = {24, 30},
  blaze_reach = 420,        -- how far around its centre a blaze looks for walls, ceiling and roof
  blaze_points = 14,        -- big flames per blaze, at most
  blazes_max = 5,           -- blazes at once, at most
  blaze_spread = 0.6,       -- heat per second a blaze gives each area next to it
  blaze_damage_reach = 130,
  embers_seconds = 90,      -- how long the embers of a blaze glow
  -- A blaze grows in stages, on the game's clock: small flames where it starts, the fire creeping
  -- over the wood, bigger flames and then dark smoke, and the full fury of the Sanctuary's fire
  -- (great clouds of fire, sparks, falling flamelets) once most of it burns and it has burned
  -- `inferno_after` seconds; it calms down `calm_before` seconds before it is out.
  smoke_after = 5,          -- seconds after a flame of the blaze starts before its dark smoke
  inferno_after = 6,
  inferno_share = 0.5,      -- the part of its flames that must be burning
  calm_before = 10,
  last_flames = {20, 30},   -- small flames left where it burned, for that many seconds
  -- What a blaze burns turns black over that many seconds, and stays black.
  char_scenery_seconds = 15,
  -- A piece of scenery bigger than this (units; a hall, a pier and its luncheonette made of one
  -- mesh) only turns black as a whole once a building has burnt down where the fire touched it
  -- (the blast, within `blast_building` of those flames) or once the fire has spread over it (its flames in at least
  -- `whole_char_cells` areas of 400 units for every 400 units of its size). A burst of the jet
  -- or a small fire in a huge hall leaves only its marks.
  whole_char_size = 900,
  whole_char_cells = 1.2,
  char_mark_spacing = 150,  -- wide burn marks on the level's own walls and floors, this far apart
  char_area_radius = 260,
  char_marks_max = 60,      -- wide burn marks a blaze leaves, at most (they stay)   -- the level's own surfaces this far around each flame of a blaze turn black
  -- Wood (and cloth) burns fast: recognised from the name of the surface's texture (trace's
  -- material) or of the object hit or its mesh; it heats faster, its blaze is longer, and the fire creeps along it (faster
  -- upwards than sideways, slowly downwards), never across what is not wood.
  wood_words = {"bois", "wood", "planch", "plank", "parquet", "caisse", "crate", "cabane", "barque",
                "ponton", "palette", "volet", "tonneau", "poutre", "escalier", "chaise", "table",
                "meub", "barriere", "cloture", "fence", "hut", "boat", "bateau", "roof", "toit",
                "porte", "door", "placard", "rideau", "curtain", "paravent"},
  wood_heat = 2.5,          -- wood heats this many times faster
  wood_blaze_seconds = 1.15, -- and burns this many times longer
  wood_creep = {up = 450, side = 240, down = 100},   -- how fast the fire creeps along wood (units/s)
  charred_seconds = 1800,   -- burnt wood stays charred this long (half an hour)
  charred_max = 80,         -- charred patches at once, at most
  -- Heat shimmer (the view waves a little) over the jet and over what burns; 0 turns it off.
  heat_strength = 1.0,
  -- A roaring fire, played once when a fire starts (it loops until the fire is out), and a
  -- crackle played again and again.
  roar_sounds = {"XIIISound.Ambient.Sanc02bFire2", "XIIISound.Ambient.SancFire"},
  burn_sounds = {"XIIISound.Ambient.SancFire", "XIIISound.Ambient.FireSpad1", "XIIISound.Ambient.FireChimney1"},
  fire_sounds = {"XIIISound.Ambient.FireSpad1", "XIIISound.Ambient.SancFire", "XIIISound.Ambient.FireChimney1"},
  sound_every = 4,      -- seconds between two crackles of a fire (the previous one is stopped)
  scorch_every = 0.08,  -- seconds between two marks while the flame touches a surface
  scorch_spacing = 70,  -- no new mark this close to one of the last ones
  fire_every = 0.35,    -- seconds between two fires left burning on the scenery
  fire_spacing = 120,   -- no new fire this close to one still burning
  fire_tongues = 4,     -- smaller flames around a fire, appearing one after the other
  fire_spread = {45, 110},  -- how far around it they appear (units, the first ones nearest)
  fire_spread_seconds = 2.5,  -- over how many seconds they appear
  fire_seconds = {8, 14},   -- how long a fire left on the scenery burns (from, to)
  fires_max = 14,       -- fires burning at once, at most
  -- Black marks stay as long as the level (nil: no end); past `marks_max`, the oldest marks of
  -- the jet go first, never those a blaze left.
  mark_seconds = nil,
  marks_max = 400,
  -- Where the flame comes out, from the eyes: forward, right, up (the tube's nozzle).
  nozzle = {42, 9, -9},
  -- The lance seen in a character's hands (third person, co-op): the tip of its model, in the
  -- frame of the gun's third-person actor (forward, right, up). The flame seen by the others
  -- is pinned there, so every machine starts it at the tip it draws.
  muzzle_tip = {104, 0, 2},
  -- Without that actor (a moment while the gun is changed): from the eyes and where he aims.
  muzzle_third = {25, 6, -62},
  -- The others' flame goes from the tip towards the point he aims at this far (units): what he
  -- aims at burns, wherever the lance in his hands points. The game holds every gun level in a
  -- character's hands, whatever the pitch of his view (only his head and chest follow it).
  aim_point = 330,
  -- The tanks on the back while the flamethrower is in hand: the bone they are put on, and their
  -- place and turn from it (game units and angles), as the game places its vest.
  tanks = {class = "Mod.FlamethrowerTanks", bone = "X Spine1", offset = {0, 14, 0}, rotation = {pitch = -16384, yaw = 0, roll = 0}},
}
local F = Flamethrower

local burning = {}   -- by character name: {actor, fx, since, carry, dead_at, panic, ...}
local players_state = {}
local scorches, last_scorch = {}, -1
local fires, last_fire = {}, -1
local hurt_props = {}   -- by actor name: when it was last hurt by the flame
local scorched_players = {}
local cells = {}         -- by area: {heat, at, normal, done, wood}
local charred = {}       -- charred patches left on burnt wood, oldest first
local blazes = {}        -- the blazes burning   -- by player name: {carry, scale} of the damage of ground fires
local marks = {}        -- black marks still on the scenery, oldest first


local add_heat   -- defined with the blazes, below

-- What a blaze will burn around a point: the scenery's meshes, doors and objects, and the
-- bodies (each once).
-- Big pieces of scenery: their size (asked once) and the areas of 400 units where flames have
-- been near them.
local sizes, big_cover, blast_spots = {}, {}, {}
local function size_of(a)
  local n = a.name
  if sizes[n] == nil then
    local ok, v = pcall(function() return a.size end)
    sizes[n] = ok and v or 0
  end
  return sizes[n]
end
-- Whether `a` may be blackened as a whole: small enough, or the fire has spread over it.
local function whole_char_ok(a)
  local size = size_of(a)
  if size <= F.whole_char_size then return true end
  local c = big_cover[a.name]
  if c == nil then return false end
  -- A building blew out where the fire touched it.
  for _, q in ipairs(blast_spots) do
    for _, at in ipairs(c.at) do
      local dx, dy, dz = q[1] - at[1], q[2] - at[2], q[3] - at[3]
      if dx * dx + dy * dy + dz * dz <= F.blast_building * F.blast_building then return true end
    end
  end
  return c.n >= size / 400 * F.whole_char_cells
end

local function near_add(b, at, radius)
  local key = string.format("%d,%d,%d", math.floor(at[1] / 400), math.floor(at[2] / 400), math.floor(at[3] / 400))
  for _, a in ipairs(actors_near(at, radius)) do
    -- Characters, and anything drawn with a mesh of the scenery: static meshes, movers, the
    -- scenes' doors and shutters (XIDCine.Porte, XIDCine.Fenetre), decorations; not the mod's
    -- own effects.
    local name = a.name
    if b.near_names[name] or a.class:find("^Mod%.") then goto skip end
    local carried = false
    pcall(function()
      carried = a:get("Owner") ~= nil or a:is_a("Inventory") or a:is_a("InventoryAttachment")
    end)
    if carried then goto skip end
    local pawn = a:is_a("Pawn")
    if not pawn then
      local ok, mesh = pcall(a.get, a, "StaticMesh")
      if not ok or mesh == nil then goto skip end
    end
    if not pawn and size_of(a) > F.whole_char_size then
      local c = big_cover[name] or {cells = {}, n = 0, at = {}}
      big_cover[name] = c
      if not c.cells[key] then c.cells[key] = true c.n = c.n + 1 c.at[#c.at + 1] = at end
    end
    b.near_names[name] = true
    b.near[#b.near + 1] = a
    ::skip::
  end
end
local blaze_stages
local spark_emitters
local blaze_kick, blaze_sounds, blaze_sounds_out
local heatable
local prop_ok = {}       -- by object name: may the heat break it

-- Plays the first sound of the list the game can play, from the flames `at` (an effect): its
-- earlier sound is stopped first, and every sound stops with it (the game stops the looping
-- sounds of an actor taken out of the game).
local playable = {}
local function play_first(list, at)
  for _, name in ipairs(list) do
    if playable[name] ~= false then
      local ok = pcall(sound, name, at)
      playable[name] = ok
      if ok then return end
    end
  end
end

-- Silent emitters at `at` that play a sound `volume` times as loud as the game plays it. The
-- game has no volume per sound: louder is the same sound played again by another emitter,
-- softer (or the part of a layer below 1) uses its distance falloff, as if the emitter were
-- further away (full volume up to SaturationDistance, then down to StabilisationVolume
-- decibels at StabilisationDistance; the emitter is about 45 units away).
local function sound_layers(name, volume, at, seconds)
  local out = {}
  local left = volume
  while left > 0.01 do
    local v = math.min(left, 1)
    left = left - v
    local h = effect{class = "Mod.PilotFlame", position = at, seconds = seconds}
    if h then
      if v < 0.999 then
        local db = -20 * math.log(math.max(v, 0.01), 10)
        h:set("SaturationDistance", 1)
        h:set("StabilisationVolume", -40)
        h:set("StabilisationDistance", 1 + 45 * 40 / db)
      end
      if pcall(sound, name, h) then out[#out + 1] = h else remove(h) end
    end
  end
  return out
end

-- A sound played once; cut after `cut` seconds of game time if given (stopped, then its
-- emitters are taken out).
local function play_once(name, volume, at, cut)
  local hs = sound_layers(name, volume, at, 4)
  if cut then
    after(cut, function()
      for _, h in ipairs(hs) do
        if h.alive then
          pcall(function() h:call("StopActorSounds") end)
          remove(h)
        end
      end
    end)
  end
end

local function play(list, at)
  pcall(function() at:call("StopActorSounds") end)
  play_first(list, at)
end

-- A fire's sounds again: its roar (from its flames) and a crackle. The game stops a looping
-- sound that is too far from the player and does not start it again by itself: played again
-- regularly, the roar is heard again when the player comes back.
local function fire_sounds_again(fx, crackle, crackle_list)
  if fx and fx.alive then play(F.roar_sounds, fx) end
  if crackle and crackle.alive then play(crackle_list, crackle) end
end

-- The sounds of a fire: its roar from its flames `fx`, its crackles from a silent emitter
-- carried with them (so that a new crackle never stops the roar). Both stop with the fire.
local function fire_sounds(fx, on)
  play_first(F.roar_sounds, fx)
  local loc = fx:get("Location")
  return effect{class = "Mod.PilotFlame", position = loc, on = on or fx}
end

local function silence(fx, crackle)
  if fx and fx.alive then pcall(function() fx:call("StopActorSounds") end) end
  if crackle and crackle.alive then
    pcall(function() crackle:call("StopActorSounds") end)
    remove(crackle)
  end
end

-- How far a sound played from `a` is heard (the game's own attenuation of an actor's sounds:
-- full volume up to SaturationDistance, then down to StabilisationVolume at
-- StabilisationDistance; a sound further than that does not start).
local function sound_range(a, r)
  if a == nil or not a.alive or r == nil then return end
  pcall(function()
    a:set("SaturationDistance", r.near)
    a:set("StabilisationDistance", r.far)
    a:set("StabilisationVolume", r.far_volume)
  end)
end

local function add(a, b, k) return {a[1] + b[1] * k, a[2] + b[2] * k, a[3] + b[3] * k} end
local function sub(a, b) return {a[1] - b[1], a[2] - b[2], a[3] - b[3]} end
local function dot(a, b) return a[1] * b[1] + a[2] * b[2] + a[3] * b[3] end
local function len(a) return math.sqrt(dot(a, a)) end
local function distance(a, b)
  local x, y, z = a[1] - b[1], a[2] - b[2], a[3] - b[3]
  return math.sqrt(x * x + y * y + z * z)
end

-- A black mark on the scenery. The game's mark (XIIIScorch) lets go of its picture after a
-- second (AbandonProjector): it is attached again, and stays as long as its actor.
local blaze_marks = 0
local function mark(class, at, n, keep)
  local m = effect{class = class, position = add(at, n, 2), facing = {-n[1], -n[2], -n[3]}, seconds = F.mark_seconds}
  if m == nil then return nil end
  if class == "Mod.CharredSurface" then m:set("DrawScale", 3.2 + 2.6 * random()) end
  m:call("AttachProjector")
  -- No end: the game's marks take themselves out after a second (LifeSpan); 0 keeps them.
  if F.mark_seconds == nil then m:set("LifeSpan", 0) end
  if keep then
    blaze_marks = blaze_marks + 1
    return m
  end
  marks[#marks + 1] = m
  while #marks + blaze_marks > F.marks_max and #marks > 0 do
    local old = table.remove(marks, 1)
    if old.alive then remove(old) end
  end
  return m
end

-- The view of a player: eyes, forward, right and up directions, and its rotation (game units).
local function view(p)
  local c = p:get("Controller")
  if c == nil then return nil end
  local r = c:get("Rotation")
  local pitch, yaw = r.pitch * math.pi / 32768, r.yaw * math.pi / 32768
  local cp, sp, cy, sy = math.cos(pitch), math.sin(pitch), math.cos(yaw), math.sin(yaw)
  local fwd = {cp * cy, cp * sy, sp}
  local right = {-sy, cy, 0}
  local up = {-sp * cy, -sp * sy, cp}
  local loc = p:get("Location")
  local eyes = {loc[1], loc[2], loc[3] + p:get("EyeHeight")}
  return {controller = c, eyes = eyes, fwd = fwd, right = right, up = up, rotation = {pitch = r.pitch, yaw = r.yaw, roll = 0}}
end

local function fuel(p)
  return p.ammo or 0
end

-- Only a soldier driven by the game's AI (IAController, which has the flight state Fuite) panics:
-- a character of a scene (driven by its scene's controller, as some soldiers are before they
-- attack) keeps doing what the level makes it do. Every character alive takes the fire's damage
-- by the game's own TakeDamage: the game decides whether it can be hurt, as for a bullet.
local function can_panic(c)
  local ok, yes = pcall(function()
    local ctrl = c:get("Controller")
    return ctrl ~= nil and ctrl:is_a("IAController")
  end)
  return ok and yes
end

-- A character catches fire, or is kept burning while the flame is on him.
local function ignite(c, by, now)
  local b = burning[c.name]
  if b == nil then
    b = {actor = c, carry = 0, by = by, since = now, sound_at = -1}
    b.fx = effect{class = "Mod.BurningBody", on = c}
    if b.fx then
      b.fx:trigger(by)
      b.crackle = fire_sounds(b.fx, c)
    end
    b.smoke = effect{class = "Mod.BlackSmoke", on = c}
    if b.smoke then b.smoke:trigger(by) end
    if F.body_sparks then
      b.sparks = effect{class = "Mod.BodySparks", on = c}
      if b.sparks then b.sparks:trigger(by) end
    end

    burning[c.name] = b
    -- Most soldiers panic: they run away from whoever burns them, with their AI's own flight
    -- (state Fuite) and its running animations. The others fight on.
    if can_panic(c) and random() < F.panic_chance then
      b.panic = true
      b.flee_at = -1
    end
  end
  b.in_flame_until = now + 0.15
  b.by = by
end

-- A soldier in panic keeps fleeing: the flight is given again when he stops (his AI pauses
-- when he cannot see the one he flees, or stops at a wall).
local function flee(b, now)
  local c = b.actor
  if now < b.flee_at then return end
  local v = c:get("Velocity")
  if b.flee_at < 0 or v[1] * v[1] + v[2] * v[2] < 60 * 60 then
    pcall(function()
      c:attack(b.by)
      c:get("Controller"):call("GotoState", "Fuite")
    end)
  end
  b.flee_at = now + 1.0
end

-- A body that has burnt long enough: its flames die out, it stays black.
local function put_out(name, b)
  if b.fx and b.fx.alive then
    b.fx:trigger(b.by)   -- the torch's second trigger: no new flames
    b.fx:set("LifeSpan", 1.5)
  end
  silence(b.fx, b.crackle)
  for _, x in ipairs({b.smoke or false, b.sparks or false}) do
    if x and x.alive then
      x:trigger(b.by)
      x:set("LifeSpan", 4)
    end
  end
  burning[name] = nil
end

local function burn(now, dt)
  for name, b in pairs(burning) do
    local c = b.actor
    -- Blacker and blacker while it burns, alive or dead (the game's corpse is the same actor).
    local ok = pcall(function() c.charred = math.min(1, (now - b.since) / F.char_seconds) end)
    if not ok then
      put_out(name, b)   -- the body was taken out of the game
    elseif not c.alive then
      if b.dead_at == nil and b.fx and b.fx.alive then
        b.fx:set("Emitters[1].Disabled", true)   -- a dead body cries no more "aahh"
      end
      b.dead_at = b.dead_at or now
      -- A dead body's sparks die down; none on a corpse that has nearly burned out.
      if b.sparks and b.sparks.alive and not b.sparks_low and now - b.dead_at > F.corpse_seconds * 0.4 then
        b.sparks_low = true
        b.sparks:trigger(b.by)
      end
      if now - b.dead_at > F.corpse_seconds then
        put_out(name, b)
      elseif b.crackle and b.crackle.alive and now - b.sound_at > F.sound_every then
        b.sound_at = now
        fire_sounds_again(b.fx, b.crackle, F.burn_sounds)
      end
    else
      if b.crackle and b.crackle.alive and now - b.sound_at > F.sound_every then
        b.sound_at = now
        fire_sounds_again(b.fx, b.crackle, F.burn_sounds)
      end
      local rate = (now < b.in_flame_until) and F.flame_damage or F.burn_damage
      -- The game multiplies a hit by its own rules (difficulty, a soldier on alert...): the
      -- health really lost is measured, so that the rates above are the ones seen.
      b.carry = b.carry + rate * dt
      local scale = b.scale or 1
      local n = math.floor(b.carry / scale)
      if n >= 1 then
        local lost = c:hurt(n, b.by, "XIII.DTFire")
        b.carry = math.max(0, b.carry - math.max(lost, n))
        if lost > 0 and c.alive then b.scale = lost / n end
      end
      if c.alive and b.panic then flee(b, now) end
      if F.heat_strength > 0 then
        pcall(heat, {at = c, radius = 70, seconds = 0.3, strength = F.heat_strength * 0.8, name = "burn " .. name})
      end
    end
  end
end

-- Who is in the flame of player p.
local function sweep(p, v, nozzle, now)
  local tan = math.tan(math.rad(F.cone))
  for _, c in ipairs(characters_near(nozzle, F.range + 100)) do
    if c.alive then
      local to = sub(c:get("Location"), nozzle)
      local along = dot(to, v.fwd)
      if along > -20 and along < F.range then
        local off = len(add(to, v.fwd, -along))
        local radius = c:get("CollisionRadius")
        if off < F.width + along * tan + radius then
          local hit = trace(nozzle, c:get("Location"), p)
          if hit == nil or hit.actor == c then ignite(c, p, now) end
        end
      end
    end
  end
end

-- The area a point is in.
local function cell_key(at)
  local c = F.blaze_cell
  return math.floor(at[1] / c) .. " " .. math.floor(at[2] / c) .. " " .. math.floor(at[3] / c)
end

-- Is a surface met by trace() wood: the name of its texture, or of the object hit?
local function is_wood(hit)
  if hit == nil or not F.wood_spread then return false end
  local names = {hit.material or ""}
  if hit.actor ~= nil then
    names[#names + 1] = hit.actor.name
    names[#names + 1] = hit.actor.class
    pcall(function() names[#names + 1] = tostring(hit.actor:get("StaticMesh")) end)
  end
  for _, name in ipairs(names) do
    local lower = name:lower()
    for _, w in ipairs(F.wood_words) do
      if lower:find(w, 1, true) then return true end
    end
  end
  return false
end

-- Heat given to the area of a point of the scenery (more on wood).
add_heat = function(at, n, amount, wood)
  local key = cell_key(at)
  local c = cells[key]
  if c == nil then c = {heat = 0} cells[key] = c end
  if c.done then return end
  c.heat = c.heat + amount * (wood and F.wood_heat or 1)
  c.at, c.normal = at, n
  c.wood = c.wood or wood
end

-- Charred wood left where a blaze burned on wood: it stays.
local function char_wood(at, n)
  if not F.charred_wood then return end
  local m = effect{class = "Mod.CharredWood", position = add(at, n, 30), facing = {-n[1], -n[2], -n[3]}, seconds = F.charred_seconds}
  if m == nil then return end
  pcall(function() m:call("AttachProjector") end)
  charred[#charred + 1] = m
  while #charred > F.charred_max do
    local old = table.remove(charred, 1)
    if old.alive then remove(old) end
  end
end

-- The points a blaze burns at: walls, ceiling and floor around the centre, and the roof (lines
-- down from above), each a little in front of its surface; none closer than 110 to another.
local function blaze_points(at, n, p)
  local from = {at[1] + n[1] * 40, at[2] + n[2] * 40, at[3] + n[3] * 40 + 30}
  local found = {}
  local function keep(h)
    if h == nil or (h.actor ~= nil and h.actor:is_a("Pawn")) then return end
    for _, q in ipairs(found) do
      if distance(q.at, h.position) < 110 then return end
    end
    found[#found + 1] = {at = add(h.position, h.normal, 20), normal = h.normal, wood = is_wood(h), hit = h.position}
  end
  keep({position = at, normal = n, material = "wood"})
  local r = F.blaze_reach
  local function probe(start, finish)
    -- Only the first points are kept. The radial loop still draws every random angle.
    if #found < F.blaze_points then keep(trace(start, finish, p)) end
  end
  for _, dz in ipairs({0, 160}) do
    for i = 0, 7 do
      local a = i * math.pi / 4 + random() * 0.3
      probe({from[1], from[2], from[3] + dz}, {from[1] + r * math.cos(a), from[2] + r * math.sin(a), from[3] + dz})
    end
  end
  -- The outer faces of the walls: lines from outside towards the centre.
  for i = 0, 5 do
    local a = i * math.pi / 3 + 0.2
    local q = {from[1] + (r + 250) * math.cos(a), from[2] + (r + 250) * math.sin(a), from[3] + 60}
    probe(q, {from[1], from[2], from[3] + 60})
  end
  -- The deck, stairs and stilts around: lines down from above, a little further out.
  for i = 0, 5 do
    local a = i * math.pi / 3 + 0.7
    local q = {from[1] + 360 * math.cos(a), from[2] + 360 * math.sin(a), from[3] + 700}
    probe(q, {q[1], q[2], from[3] - 400})
  end
  probe(from, {from[1], from[2], from[3] + r})                             -- the ceiling
  probe({from[1], from[2], from[3] + 900}, from)                           -- the roof, from above
  for i = 0, 3 do                                                          -- the slopes of the roof
    local a = i * math.pi / 2 + 0.4
    local q = {from[1] + 180 * math.cos(a), from[2] + 180 * math.sin(a), from[3] + 900}
    probe(q, {q[1], q[2], from[3] - 100})
  end
  while #found > F.blaze_points do table.remove(found) end
  return found
end

local function ignite_area(c, p, now)
  c.done = true
  local seconds = F.blaze_seconds[1] + random() * (F.blaze_seconds[2] - F.blaze_seconds[1])
  if c.wood then seconds = seconds * F.wood_blaze_seconds end
  local b = {spots = {}, by = p, until_t = now + seconds, at = c.at, wood = c.wood, burnt = {}, started = now, stage = 1}
  local points = blaze_points(c.at, c.normal, p)
  b.points = points
  -- What it will burn: the scenery's meshes, doors and objects, and the bodies around.
  if F.char_scenery then
    b.near, b.near_names, b.near_spots = {}, {}, 0
    near_add(b, c.at, F.blaze_reach + 400)
  end
  for i, spot in ipairs(points) do
    -- On wood the fire creeps from where it started: each point catches after the time the
    -- flames take to get there along the wood (quick upwards, slow downwards); what is not wood
    -- does not catch. Elsewhere the points catch one after the other.
    local delay = 0.4 * (i - 1) + random() * 0.3
    if c.wood then
      local d = sub(spot.at, c.at)
      local side = math.sqrt(d[1] * d[1] + d[2] * d[2])
      local cr = F.wood_creep
      delay = side / cr.side + (d[3] > 0 and d[3] / cr.up or -d[3] / cr.down) + random() * 0.5
    end
    if c.wood and not spot.wood and i > 1 then goto skip end
    -- A wood fire also heats the wood it reaches far from its centre: it goes on creeping there.
    if c.wood and spot.wood and distance(spot.at, c.at) > F.blaze_cell * 0.6 then
      after(delay + 3, function() add_heat(spot.at, spot.normal, F.blaze_heat, true) end)
    end
    after(delay, function()
      if time() > b.until_t then return end
      -- As the fire takes hold, the flames grow: on wood, the points it reaches later (and the
      -- roof) burn with the Sanctuary's tallest flames.
      local class = "Mod.Blaze"
      if F.tall_flames and c.wood and (i > #points / 2 or spot.normal[3] > 0.5) and i % 2 == 0 then class = "Mod.BlazeTall" end
      local fx = effect{class = class, position = spot.at}
      if fx then
        fx:trigger(p)
        -- Its dark smoke comes once the flames have taken hold.
        fx:set("Emitters[1].Disabled", true)
        after(F.smoke_after, function() if fx.alive and time() < b.until_t then fx:set("Emitters[1].Disabled", false) end end)
        b.spots[#b.spots + 1] = fx
        b.spot_points = b.spot_points or {}
        b.spot_points[#b.spots] = spot
        mark("Mod.FireScorch", spot.at, spot.normal, true)
        b.extra = b.extra or {}
        local n = spot.normal
        if F.wall_glow then
          local g = effect{class = "Mod.FireGlow", position = add(spot.at, n, -18), facing = {-n[1], -n[2], -n[3]}, seconds = b.until_t - time() + 2}
          if g then pcall(function() g:call("AttachProjector") end) b.extra[#b.extra + 1] = g end
        end
        -- Fine sparks while the fire spreads: a third of them.
        if F.sparks and F.particle_amount > 0 and i % 3 == 1 then
          spark_emitters(b, spot, false)
        end
        if F.heat_strength > 0 then
          pcall(heat, {at = fx, radius = 200, seconds = b.until_t - time(), strength = F.heat_strength})
        end
        if b.wood and spot.wood then b.burnt[#b.burnt + 1] = spot end
      end
    end)
    ::skip::
  end
  -- A big blaze (on wood, or with many flames) also raises a tall column of smoke above its highest
  -- point, once it has taken hold.
  if F.smoke_column and (c.wood or #points >= 6) then
    local top = points[1]
    for _, sp in ipairs(points) do if sp.at[3] > top.at[3] then top = sp end end
    after(4 + random() * 2, function()
      if time() > b.until_t then return end
      local col = effect{class = "Mod.SmokeColumn", position = top.at}
      b.extra = b.extra or {}
      if col then
        col:trigger(p)
        b.extra[#b.extra + 1] = col
      end
      -- Embers flying away above the roof, and floating inside where it started.
      for _, at in ipairs({top.at, points[1].at}) do
        local fe = effect{class = "Mod.FlyingEmbers", position = at}
        if fe then fe:trigger(p) b.extra[#b.extra + 1] = fe end
      end
    end)
  end
  blazes[#blazes + 1] = b
  log("flamethrower: a blaze starts")
end

-- The fire spreads: smaller tongues of flame appear around it, one after the other and a little
-- further each time, on the same surface (a line along it finds where it is; none over an edge).
local function spread_fire(f, at, n, prop, p)
  -- Two directions along the surface.
  local t1 = math.abs(n[3]) < 0.9 and {-n[2], n[1], 0} or {1, 0, 0}
  local l1 = len(t1)
  t1 = {t1[1] / l1, t1[2] / l1, t1[3] / l1}
  local t2 = {n[2] * t1[3] - n[3] * t1[2], n[3] * t1[1] - n[1] * t1[3], n[1] * t1[2] - n[2] * t1[1]}
  local turn = random() * 2 * math.pi
  for i = 1, F.fire_tongues do
    after(F.fire_spread_seconds * (i - 0.5) / F.fire_tongues, function()
      if not f.fx.alive or time() > f.until_t then return end
      local a = turn + i * 2 * math.pi / F.fire_tongues + (random() - 0.5)
      local d = F.fire_spread[1] + (F.fire_spread[2] - F.fire_spread[1]) * (i - 1) / math.max(1, F.fire_tongues - 1)
      local q = add(add(at, t1, d * math.cos(a)), t2, d * math.sin(a))
      local hit = trace(add(q, n, 30), add(q, n, -40), p)
      if hit == nil then return end
      local m = hit.normal
      local on = prop ~= nil and prop.alive and prop or nil
      local ok, t = pcall(effect, {class = "Mod.SceneryTongue", position = add(hit.position, m, 4), on = on})
      if ok and t then
        t:trigger(p)
        f.tongues[#f.tongues + 1] = t
      end
    end)
  end
end

-- A black mark where the flame meets a wall or a floor.
local function scorch(p, v, nozzle, now)
  if now - last_scorch < F.scorch_every then return end
  last_scorch = now
  local spread = math.rad(F.cone) * 0.6
  local dir = add(add(v.fwd, v.right, (random() * 2 - 1) * spread), v.up, (random() * 2 - 1) * spread)
  local hit = trace(nozzle, add(nozzle, dir, F.range * 0.95), p)
  if hit == nil then return end
  -- Characters burn by themselves (sweep); anything else the flame meets is scenery: the
  -- level, its static meshes (a pier, a wall of crates), its movers and objects.
  local prop = hit.actor
  if prop ~= nil and prop:is_a("Pawn") then return end
  if prop ~= nil then
    -- Objects burn too: they get darker, and the flame hurts them (a crate breaks, a barrel blows up).
    -- (A big piece of scenery only keeps the marks: it turns black once the fire spreads over it.)
    if whole_char_ok(prop) then pcall(function() prop.charred = math.min(1, prop.charred + 0.08) end) end
    local last = hurt_props[prop.name] or -1
    if now - last > 0.25 then
      hurt_props[prop.name] = now
      pcall(function() prop:hurt(math.floor(F.prop_damage * 0.25 + 0.5), p, "XIII.DTFire", hit.position) end)
    end
  end
  -- The flame heats the area it touches (a blaze when it is hot enough).
  hit.wood = is_wood(hit)
  add_heat(hit.position, hit.normal, F.scorch_every, hit.wood)
  for i = math.max(1, #scorches - 12), #scorches do
    if distance(scorches[i], hit.position) < F.scorch_spacing then return end
  end
  local n = hit.normal
  if mark("Mod.FlameScorch", hit.position, n) then
    scorches[#scorches + 1] = hit.position
    if #scorches > 200 then table.remove(scorches, 1) end
  end
  -- Now and then the scenery catches fire there: flames and smoke for a few seconds, then a
  -- wider black mark.
  if now - last_fire < F.fire_every or #fires >= F.fires_max then return end
  for _, f in ipairs(fires) do
    if distance(f.at, hit.position) < F.fire_spacing then return end
  end
  last_fire = now
  -- On an object, the fire moves with it (a boat, a door). The flame may have just destroyed it
  -- (a crate breaks): then the fire stays where it was.
  if prop ~= nil and not prop.alive then prop = nil end
  local ok, fx = pcall(effect, {class = "Mod.SceneryFire", position = add(hit.position, n, 6), on = prop})
  if not ok then fx = effect{class = "Mod.SceneryFire", position = add(hit.position, n, 6)} end
  if fx then
    fx:trigger(p)
    local seconds = F.fire_seconds[1] + random() * (F.fire_seconds[2] - F.fire_seconds[1])
    local f = {fx = fx, crackle = fire_sounds(fx), at = hit.position, normal = n, until_t = now + seconds, by = p, sound_at = -1, tongues = {}}
    f.smoke = effect{class = "Mod.BlackSmoke", position = add(hit.position, n, 6), on = prop}
    if f.smoke then f.smoke:trigger(p) end
    if F.heat_strength > 0 then
      pcall(heat, {at = add(hit.position, n, 40), radius = 110, seconds = seconds, strength = F.heat_strength * 0.8})
    end
    fires[#fires + 1] = f
    mark("Mod.FireScorch", hit.position, n)
    spread_fire(f, hit.position, n, prop, p)
  end
end

-- Health lost by a player standing in a ground fire, counted on the game's clock; the game's own
-- rules change a hit (difficulty...): the health really lost is measured, as for characters.
local function scorch_player(p, by, dt)
  local d = scorched_players[p.name]
  if d == nil then d = {carry = 0} scorched_players[p.name] = d end
  d.carry = d.carry + F.ground_fire_damage * dt
  -- The game gives a player only a part of a hit (about 15 %, more in harder modes): a first
  -- guess, then what is measured; hits of 2 health or more, so that nothing rounds to 0.
  local scale = d.scale or 0.15
  local n = math.floor(d.carry / scale)
  if d.carry >= 2 and n >= 1 then
    local lost = p:hurt(n, by, "XIII.DTFire")
    d.carry = math.max(0, d.carry - math.max(lost, n))
    if lost > 0 and p.alive then d.scale = lost / n end
  end
end

-- The full fury of a blaze, once most of it burns and it has burned long enough: the
-- Sanctuary's great clouds of fire, its sparks and falling flamelets. They fade in (the emitters'
-- own fade), and stop coming when the blaze calms down before it is out (their last ones fade).
-- Where a blaze leaves wide burn marks on the level's walls, floors and ceilings: lines from its
-- centre and from its flames in every direction, every hit at least `char_mark_spacing` apart.
local function char_targets(b)
  local out = {}
  local function keep(h)
    if h == nil or h.actor ~= nil then return end
    for _, q in ipairs(out) do
      if distance(q.at, h.position) < F.char_mark_spacing then return end
    end
    out[#out + 1] = {at = h.position, normal = h.normal}
  end
  local from = {b.at}
  for k, spot in ipairs(b.spot_points or {}) do
    if k % 2 == 1 then from[#from + 1] = add(spot.at, spot.normal, 30) end
  end
  local dirs = {{0, 0, -1}, {0, 0, 1}}
  for i = 0, 7 do
    local a = i * math.pi / 4
    dirs[#dirs + 1] = {math.cos(a), math.sin(a), 0}
    dirs[#dirs + 1] = {math.cos(a) * 0.7, math.sin(a) * 0.7, -0.7}
  end
  for _, f in ipairs(from) do
    for _, d in ipairs(dirs) do
      keep(trace(f, add(f, d, F.blaze_reach), b.by))
    end
  end
  -- The edge of the burnt floor around each flame: round marks laid on it, so that no straight
  -- line shows between burnt and unburnt planks.
  for k, spot in ipairs(b.spot_points or {}) do
    if k % 2 == 1 then
      for i = 0, 5 do
        local a = i * math.pi / 3 + k
        local r = F.char_area_radius * (0.75 + 0.2 * random())
        local q = {spot.at[1] + r * math.cos(a), spot.at[2] + r * math.sin(a), spot.at[3] + 120}
        keep(trace(q, {q[1], q[2], q[3] - 500}, b.by))
      end
    end
  end
  while #out > F.char_marks_max do table.remove(out, math.random(#out)) end
  return out
end

-- The fine sparks of a blaze at one of its flames: a rain of sparks under what burns above (a
-- ceiling, a roof's underside, a beam), and sparks rising above it into the air. While the fire
-- spreads (`full` false) a third of `particle_amount`, in its fury all of it; each emitter is
-- the Sanctuary's own density, so the amount is a number of emitters at that flame.
local spark_count = {}
spark_emitters = function(b, spot, full)
  local n = F.particle_amount * (full and 1 or 1 / 3)
  local key = tostring(b) .. " " .. spot.at[1] .. " " .. spot.at[2] .. " " .. spot.at[3]
  local made = spark_count[key] or 0
  local want = math.floor(n + 0.5)
  if not full and want < 1 then want = 1 end
  b.fury = b.fury or {}
  for _ = made + 1, want do
    -- Raining down: under a ceiling or a roof (a surface facing down), or from above any other.
    local rain_at = spot.normal[3] < -0.3 and add(spot.at, spot.normal, 10) or {spot.at[1], spot.at[2], spot.at[3] + 160}
    local rise_at = {spot.at[1], spot.at[2], spot.at[3] + 120}
    for _, e in ipairs({{"Mod.SparkRain", rain_at}, {"Mod.SparkRise", rise_at}}) do
      local x = effect{class = e[1], position = e[2]}
      if x then
        x:trigger(b.by)
        if full then b.fury[#b.fury + 1] = x else b.extra[#b.extra + 1] = x end
      end
    end
  end
  spark_count[key] = math.max(made, want)
end

local function fury(b, on)
  if on then
    b.fury = {}
    if F.char_scenery then b.char_marks = char_targets(b) end
    -- Spread over a couple of seconds: not every emitter of the fury in the same frame.
    for k, fx in ipairs(b.spots) do
      after(0.12 * k, function()
        local spot = b.spot_points[k]
        if b.stage == 2 and spot and fx.alive then
          local classes = {}
          if F.fire_clouds and k % 5 == 1 then classes[#classes + 1] = "Mod.FireCloud" end
          if F.sparks and k % 4 == 1 then
            classes[#classes + 1] = "Mod.Sparks"
            classes[#classes + 1] = "Mod.Flamelets"
          end
          if F.sparks and F.particle_amount > 0 then spark_emitters(b, spot, true) end
        if F.fury_smoke and k % 4 == 2 then classes[#classes + 1] = "Mod.FurySmoke" end
        if F.ceiling_smoke and spot.normal[3] < -0.5 then classes[#classes + 1] = "Mod.CeilingSmoke" end
          for _, class in ipairs(classes) do
            local at = spot.at
            if class == "Mod.CeilingSmoke" then
            at = add(spot.at, spot.normal, 50)
          elseif class == "Mod.FireCloud" then
              -- Set back, towards the middle of the blaze and above it: volume behind the
              -- upright flames, partly hidden by them.
              at = {spot.at[1] * 0.4 + b.at[1] * 0.6, spot.at[2] * 0.4 + b.at[2] * 0.6, math.max(spot.at[3], b.at[3]) + 180}
            end
            local x = effect{class = class, position = at}
            if x then
              x:trigger(b.by)
              b.fury[#b.fury + 1] = x
            end
          end
        end
      end)
    end
  else
    for _, x in ipairs(b.fury or {}) do
      if x.alive then
        pcall(function() x:trigger(b.by) end)
        x:set("LifeSpan", 13)
      end
    end
    b.fury = {}
  end
end

-- Everything the blaze burns gets blacker as it burns, and stays black: the scenery's meshes
-- (walls, roof, floor, furniture), its doors and objects, and the bodies lying there. A body
-- only looks burnt: nothing more is done to it. A character alive and standing (with its AI)
-- burns as the flame burns it, not here.
local function char_around(b, now)
  if not F.char_scenery or b.near == nil or now - (b.char_at or -1) < 0.5 then return end
  b.char_at = now
  -- What is near each flame of the blaze burns too (a curtain, a table, a door), looked for again
  -- now and then: a window that breaks is replaced by its broken pieces.
  local spots = b.spot_points or {}
  if now - (b.near_at or -100) > 8 then
    b.near_at = now
    b.near_spots = 0
  end
  -- Two flames at a time (looking around is not free): every flame is visited in a few seconds.
  local last = math.min(#spots, b.near_spots + 2)
  for k = b.near_spots + 1, last do near_add(b, spots[k].at, F.char_area_radius + 400) end
  b.near_spots = last
  -- The level's own walls, floors and ceilings (not actors) get wide burn marks, a few at a
  -- time while the blaze rages.
  for _ = 1, 3 do
    local q = table.remove(b.char_marks or {}, 1)
    if q == nil then break end
    mark("Mod.CharredSurface", q.at, q.normal, true)
  end
  local k = math.min(1, (now - b.started) / F.char_scenery_seconds)
  -- The level's own floors, walls and ceilings around the blaze and its flames, darker by
  -- steps of an eighth (each step makes the game look for those surfaces again).
  local step = math.floor(k * 8) / 8
  if step > (b.char_step or 0) then
    b.char_step = step
    char_area(b.at, F.char_area_radius * 1.4, step)
    for _, spot in ipairs(b.spot_points or {}) do char_area(spot.at, F.char_area_radius, step) end
  end
  for _, a in ipairs(b.near) do
    pcall(function()
      if a:is_a("Pawn") and a.alive and burning[a.name] == nil and a:get("Controller") ~= nil then return end
      if not whole_char_ok(a) then return end
      if a.charred < k then a.charred = k end
    end)
  end
end

-- The sound of a blaze: silent carriers at its middle, one per layer, each playing its loop with
-- its own reach. Every few seconds each layer is played again (the game drops a looping sound too
-- far from the player and does not start it again), the layers of the stage come in and the
-- others are let go: a new one starts before the old one stops, so there is no gap.
local function carrier(at, r, seconds)
  local h = effect{class = "Mod.PilotFlame", position = at, seconds = seconds}
  if h then sound_range(h, r) end
  return h
end

local function play_on(h, name)
  if h and h.alive then
    pcall(function() h:call("StopActorSounds") end)
    pcall(sound, name, h)
  end
end

local function in_stage(list, stage)
  for _, s in ipairs(list) do if s == stage then return true end end
  return false
end

local function let_go(h, delay)
  if h == nil then return end
  after(delay or 1.5, function()
    if h.alive then
      pcall(function() h:call("StopActorSounds") end)
      remove(h)
    end
  end)
end

blaze_kick = function(b)
  local k = F.blaze_kick
  local h = carrier({b.at[1], b.at[2], b.at[3] + 100}, k, 8)
  if h then pcall(sound, k.sound, h) end
end

blaze_sounds = function(b, now)
  if now - (b.sound_at or -100) < F.sound_every * 2 then return end
  b.sound_at = now
  b.layers = b.layers or {}
  local mid = {b.at[1], b.at[2], b.at[3] + 100}
  for _, L in ipairs(F.blaze_layers) do
    local held = b.layers[L.name] or {}
    if in_stage(L.stages, b.stage) then
      -- New carriers each time the reach could change; the old ones are let go a moment later.
      for _, h in ipairs(held) do let_go(h) end
      held = {}
      for _ = 1, math.max(1, L.loud or 1) do
        local h = carrier(mid, L)
        if h then play_on(h, L.sound) held[#held + 1] = h end
      end
    else
      for _, h in ipairs(held) do let_go(h, 2.5) end
      held = {}
    end
    b.layers[L.name] = held
  end
  local C = F.blaze_crackles
  local c = C[b.stage]
  if c then
    let_go(b.crackle)
    b.crackle = carrier(mid, c)
    play_on(b.crackle, c.sound)
  end
  if b.stage == 2 and b.fury_at and now - b.fury_at >= C.peak.after then
    let_go(b.peak)
    b.peak = carrier(mid, C.peak)
    play_on(b.peak, C.peak.sound)
  elseif b.peak then
    let_go(b.peak)
    b.peak = nil
  end
end

-- The blaze is out: every layer stops (a moment later, so that its last flames are heard dying),
-- and the crackle of its embers goes on a while.
blaze_sounds_out = function(b)
  for _, held in pairs(b.layers or {}) do
    for _, h in ipairs(held) do let_go(h, 3) end
  end
  let_go(b.crackle, 3)
  let_go(b.peak, 1)
  local o = F.blaze_crackles.out
  if o and o.seconds > 0 then
    local h = carrier({b.at[1], b.at[2], b.at[3] + 40}, o, o.seconds)
    if h then
      play_on(h, o.sound)
      after(o.seconds - 0.5, function() if h.alive then pcall(function() h:call("StopActorSounds") end) end end)
    end
  end
end

-- The blast: the fire bursts out of the building's openings on every side. Openings: the windows
-- and doors around (the game's movers that break or open: Fenetre, BreakableMover, Porte...) and,
-- where there are none, the directions in which a line from the middle goes out without meeting
-- a wall. Everything breakable around breaks (as the heat breaks it, all at once); characters
-- close by are hurt and pushed by the game's grenade damage.
local function blast(b, now, bd)
  b.blasted = true
  local mid = {b.at[1], b.at[2], b.at[3] + 80}
  log("flamethrower: a blaze blows out")
  local boom = effect{class = "XIII.GrenadExplosionEmitter", position = mid}
  for _, snd in ipairs(F.blast_sounds) do
    after(snd[2], function()
      local h = effect{class = "Mod.PilotFlame", position = mid, seconds = 8}
      if h == nil then return end
      local r = F.blast_sound_range
      if (snd[3] or 0) > 0 then
        -- Softer than the first by that many decibels: the game has no volume per sound, so the
        -- level the first one has for the nearest player (the game's attenuation curve,
        -- 20 log10(step / 127) between `near` and `far`) is computed here, lowered, and given to
        -- this one as a level that does not change with distance.
        local d = math.huge
        for _, pl in ipairs(players()) do d = math.min(d, len(sub(pl:get("Location"), mid))) end
        local db = r.far_volume
        if d <= r.near then
          db = 0
        elseif d < r.far then
          local step = math.floor((r.far - d) / (r.far - r.near) * 126) + 1
          db = 20 * math.log(step / 127, 10)
        end
        r = {near = 0.5, far = 1, far_volume = math.max(-60, db - snd[3])}
      end
      sound_range(h, r)
      pcall(sound, snd[1], h)
    end)
  end
  pcall(heat, {at = mid, radius = 600, seconds = 1.5, strength = 2})
  -- Openings.
  local outs = {}
  for _, a in ipairs(actors_near(mid, F.blaze_reach + 250, "Mover")) do
    local ok, mesh = pcall(a.get, a, "StaticMesh")
    if ok and mesh ~= nil then outs[#outs + 1] = a:get("Location") end
  end
  if #outs < 4 then
    for i = 0, 7 do
      local ang = i * math.pi / 4
      local to = add(mid, {math.cos(ang), math.sin(ang), 0}, F.blaze_reach + 300)
      local h = trace(mid, to, b.by)
      if h == nil then outs[#outs + 1] = add(mid, {math.cos(ang), math.sin(ang), 0}, F.blaze_reach * 0.6)
      elseif h.material and tostring(h.material):lower():find("verre") then outs[#outs + 1] = h.position end
    end
  end
  for k, o in ipairs(outs) do
    after(0.05 * k + random() * 0.2, function()
      local d = sub(o, mid)
      d[3] = 0
      local l = len(d)
      if l < 1 then return end
      local f = {d[1] / l, d[2] / l, 0.15}
      local burst = effect{class = "Mod.WindowBurst", position = add(o, f, -10), facing = f, seconds = 4}
      if burst then burst:trigger(b.by) end
    end)
  end
  -- The whole building catches: every part of it burns black, and new blazes start inside and
  -- along its outer walls (lines from the middle outwards find them).
  if bd then
    for _, a in ipairs(bd.parts) do
      pcall(function() if a.charred < 0.6 then a.charred = 0.6 end end)
    end
    for _, a in ipairs(bd.parts) do
      after(2 + random() * 6, function() pcall(function() a.charred = 1 end) end)
    end
  end
  for i = 0, 5 do
    local ang = i * math.pi / 3 + 0.3
    local h = trace(mid, add(mid, {math.cos(ang), math.sin(ang), 0}, F.blaze_reach + 400), b.by)
    if h then
      local key = cell_key(h.position)
      local c = cells[key]
      if c == nil then c = {heat = 0} cells[key] = c end
      if not c.done then
        c.at, c.normal, c.wood = h.position, h.normal, true
        c.heat = F.blaze_heat
      end
    end
  end
  -- Everything breakable around breaks: windows (those the level uses as a signal too, if
  -- `blast_breaks_all`), shutters, bottles, glasses, the radio...
  for _, a in ipairs(actors_near(mid, F.blaze_reach + 300)) do
    local breakable = heatable(a)
    if not breakable and F.blast_breaks_all then
      pcall(function()
        breakable = (a:is_a("XIIIDecoPickup")
          or (a:is_a("BreakableMover") and a:get("BreakType") ~= 1 and not a:get("bBroken")))
          and a:get("Owner") == nil
      end)
    end
    if breakable then
      after(random() * 0.6, function()
        pcall(function() a:hurt(200, b.by, "XIII.DTFire") end)
        prop_ok[a.name] = false
      end)
    end
  end
  -- Characters and players close by: hurt and pushed outwards.
  local who = characters_near(mid, F.blast_reach)
  for _, pl in ipairs(players()) do who[#who + 1] = pl end
  for _, c in ipairs(who) do
    pcall(function()
      local l = c:get("Location")
      local d = sub(l, mid)
      local dist = len(d)
      if dist < F.blast_reach and c.alive then
        local k = 1 - dist / F.blast_reach
        local push = add({0, 0, 0}, {d[1] / math.max(dist, 1), d[2] / math.max(dist, 1), 0.4}, 60000 * k)
        c:hurt(math.max(1, math.floor(F.blast_damage * k)), b.by, "XIII.DTGrenaded", mid, push)
      end
    end)
  end
end

-- The buildings that burn: the scenery around the first blaze on wood (its meshes, doors,
-- shutters, furniture). The blast comes when `blast_share` of them burn or are burnt.
local buildings = {}
local function building_of(b)
  for _, bd in ipairs(buildings) do
    if distance(bd.at, b.at) < F.blast_building then return bd end
  end
  local bd = {at = b.at, parts = {}, blown = false, samples = {}}
  -- Places on the building (its walls, floor and roof, inside and out) found by lines from the
  -- first blaze: the share of them that a flame has reached is how much of it burns.
  local mid = {b.at[1], b.at[2], b.at[3] + 60}
  for _, dz in ipairs({0, 150}) do
    for i = 0, 11 do
      local ang = i * math.pi / 6
      local h = trace({mid[1], mid[2], mid[3] + dz}, add({mid[1], mid[2], mid[3] + dz}, {math.cos(ang), math.sin(ang), 0}, F.blast_building), b.by)
      if h then bd.samples[#bd.samples + 1] = {at = h.position} end
    end
  end
  for _, d in ipairs({{0, 0, 1}, {0, 0, -1}}) do
    local h = trace(mid, add(mid, d, F.blast_building), b.by)
    if h then bd.samples[#bd.samples + 1] = {at = h.position} end
  end
  for _, a in ipairs(actors_near(b.at, F.blast_building)) do
    pcall(function()
      if (a:is_a("StaticMeshActor") or a:is_a("Mover") or a:is_a("Decoration"))
          and not a.class:find("^Mod%.") and a:get("Owner") == nil and a:get("StaticMesh") ~= nil then
        bd.parts[#bd.parts + 1] = a
      end
    end)
  end
  buildings[#buildings + 1] = bd
  return bd
end

local function burnt_share(bd)
  if #bd.samples < 6 then return 0 end
  local n = 0
  for _, sp in ipairs(bd.samples) do
    if not sp.burnt then
      for _, b in ipairs(blazes) do
        for _, q in ipairs(b.spot_points or {}) do
          if distance(q.at, sp.at) < F.blast_reach_sample then sp.burnt = true break end
        end
        if sp.burnt then break end
      end
    end
    if sp.burnt then n = n + 1 end
  end
  return n / #bd.samples
end

local blasts = {}   -- where buildings blew out
blaze_stages = function(b, now)
  if F.blast and b.wood then
    b.building = b.building or building_of(b)
    local bd = b.building
    bd.started = bd.started or now
    if not bd.blown and b.stage == 2 and now - bd.started >= F.blast_min_seconds and now - (bd.checked or -100) > 1 then
      bd.checked = now
      bd.share = burnt_share(bd)
      if bd.share >= F.blast_share then
        bd.blown = true
        b.blasted = true
        blasts[#blasts + 1] = b.at
        blast_spots[#blast_spots + 1] = b.at
        log(string.format("flamethrower: building %.0f%% burnt (%d places), it blows out", bd.share * 100, #bd.samples))
        blast(b, now, bd)
      end
    end
  end
  local share = #b.spots / math.max(1, #b.points)
  if b.stage < 2 and share >= F.inferno_share and now - b.started >= F.inferno_after and now < b.until_t - F.calm_before then
    b.stage = 2
    b.fury_at = now
    b.sound_at = 0
    blaze_kick(b)
    fury(b, true)
  elseif b.stage == 2 and now >= b.until_t - F.calm_before then
    b.stage = 3
    b.sound_at = 0
    fury(b, false)
    -- It dies down: its big flames go out one after the other over its last seconds, and small
    -- flames stay a while where they burned, then only smoke.
    for k, fx in ipairs(b.spots) do
      local spot = b.spot_points and b.spot_points[k]
      after(random() * (F.calm_before - 2), function()
        if fx.alive then
          pcall(function() fx:trigger(b.by) end)
          fx:set("LifeSpan", 3)
        end
        if spot and k % 2 == 1 then
          local t = effect{class = "Mod.SceneryTongue", position = spot.at, seconds = F.last_flames[1] + random() * (F.last_flames[2] - F.last_flames[1])}
          if t then t:trigger(b.by) end
        end
      end)
    end
  end
  char_around(b, now)
end

-- Blazes: hot areas burst into flames; a blaze heats its neighbours, then dies down to embers.
local function blazes_step(now, dt)
  -- Fires burning on the scenery heat their area.
  for _, f in ipairs(fires) do
    if f.fx.alive then add_heat(f.at, f.normal or {0, 0, 1}, 0.2 * dt) end
  end
  for i = #blazes, 1, -1 do
    local b = blazes[i]
    if now < b.until_t then
      blaze_stages(b, now)
      blaze_sounds(b, now)
      local c = F.blaze_cell
      for _, d in ipairs({{c, 0, 0}, {-c, 0, 0}, {0, c, 0}, {0, -c, 0}, {0, 0, c}}) do
        local q = {b.at[1] + d[1], b.at[2] + d[2], b.at[3] + d[3]}
        local cell = cells[cell_key(q)]
        if cell and cell.at then cell.heat = cell.heat + F.blaze_spread * dt * ((b.wood and cell.wood) and F.wood_heat or 1) end
      end
    else
      -- Out: its flames die out, embers glow where it burned, burnt wood stays charred.
      for _, spot in ipairs(b.burnt or {}) do char_wood(spot.at, spot.normal) end
      for k, fx in ipairs(b.spots) do
        if fx.alive then
          local at = fx:get("Location")
          fx:trigger(b.by)
          fx:set("LifeSpan", 4)
          if F.embers then
            local e = effect{class = "Mod.Embers", position = at, seconds = F.embers_seconds}
            if e then e:trigger(b.by) end
          end
        end
      end
      for _, x in ipairs(b.extra or {}) do
        if x.alive then
          pcall(function() x:trigger(b.by) end)
          x:set("LifeSpan", 2)
        end
      end
      fury(b, false)
      b.char_at = nil
      char_around(b, math.huge)
      -- Out: whatever burn marks are left to make are made now.
      if F.char_scenery then
        if b.char_marks == nil then b.char_marks = char_targets(b) end
        for _, q in ipairs(b.char_marks) do mark("Mod.CharredSurface", q.at, q.normal, true) end
        b.char_marks = {}
      end
      blaze_sounds_out(b)
      -- Smoke still rising from where it burned, a good while.
      if F.after_smoke_seconds > 0 then
        for k, spot in ipairs(b.spot_points or {}) do
          if k % 3 == 1 then
            local e = effect{class = "Mod.AfterSmoke", position = spot.at, seconds = F.after_smoke_seconds}
            if e then
              e:trigger(b.by)
              -- It thins out and stops a little before it goes.
              after(F.after_smoke_seconds - 10, function() if e.alive then pcall(function() e:trigger(b.by) end) end end)
            end
          end
        end
      end
      table.remove(blazes, i)
    end
  end
  if #blazes >= F.blazes_max then return end
  for _, c in pairs(cells) do
    if not c.done and c.heat >= F.blaze_heat and c.at then
      local p = players()[1]
      if p then ignite_area(c, p, now) end
      return
    end
  end
end

-- The objects a fire breaks around a point: bottles and glasses, windows, shutters and panels,
-- the way a bullet breaks them; not what someone holds, and none whose breaking sets off an
-- event of the level (its Event is set).
-- An event that only sets off effects (the radio's sparks: its emitters) changes nothing in the
-- level's story; one that reaches anything else (a trigger, a scene, a door) is left alone.
local events_seen = {}
local function harmless_event(event)
  local known = events_seen[event]
  if known ~= nil then return known end
  local ok, yes = pcall(function()
    local any = false
    for _, a in ipairs(actors_near(players()[1], 1e6)) do
      if tostring(a:get("Tag")) == event then
        any = true
        if not a:is_a("Emitter") then return false end
      end
    end
    return any
  end)
  events_seen[event] = ok and yes or false
  return events_seen[event]
end

heatable = function(a)
  local name = a.name
  local known = prop_ok[name]
  if known ~= nil then return known end
  local ok, yes = pcall(function()
    -- Only these classes break this way. Looking up an event can scan the whole map.
    local pickup = a:is_a("XIIIDecoPickup")
    if not pickup and not a:is_a("BreakableMover") then return false end
    if a.class:find("^Mod%.") or a:get("Owner") ~= nil then return false end
    local event = tostring(a:get("Event") or "None")
    if event ~= "None" and event ~= "" and not harmless_event(event) then return false end
    -- Bottles and glasses (XIIIDecoPickup): the game breaks them at the first hit.
    if pickup then return true end
    -- Windows, shutters and panels (XIDCine.BreakableMover, Fenetre): they break when their
    -- health is gone, unless the level made them unbreakable (BreakType 1) or they are broken.
    return a:get("BreakType") ~= 1 and not a:get("bBroken") and (a:get("Health") or 0) > 0
  end)
  prop_ok[name] = ok and yes or false
  return prop_ok[name]
end

local heat_carry = {}   -- by object name: damage counted, not given yet
local heat_one
local function heat_around(at, reach, rate, by, dt)
  for _, a in ipairs(actors_near(at, reach)) do
    if a.alive ~= false and heatable(a) then
      local l = a:get("Location")
      heat_one(a, l, distance(l, at), reach, rate, by, dt)
    end
  end
end

-- Heat given to one object `d` away from the fire.
heat_one = function(a, l, d, reach, rate, by, dt)
  do
    do
      local k = 1 - d / reach
      if k > 0 then
        -- A little random each time, so that things do not all break at once.
        local c = (heat_carry[a.name] or random() * 3) + rate * k * dt * (0.5 + random())
        if c >= 3 then
          local n = math.floor(c)
          c = c - n
          local before = nil
          pcall(function() before = a:get("Health") end)
          pcall(function() a:hurt(n, by, "XIII.DTFire", l) end)
          -- Broken (or gone), or kept whole by the level: left alone from then on.
          pcall(function()
            if before == nil or a.alive == false or a:get("bBroken") or (a:get("Health") or 0) <= 0 or a:get("Health") == before then
              prop_ok[a.name] = false
            end
          end)
        end
        heat_carry[a.name] = c
      end
    end
  end
end

local heat_at = -1
local function heat_props(now, dt)
  if now - heat_at < 0.4 then return end
  dt = math.min(1, now - heat_at)
  heat_at = now
  local H = F.heat_props
  for _, f in ipairs(fires) do
    if f.fx.alive then heat_around(f.at, H.fire_reach, H.fire_rate, f.by, dt) end
  end
  -- A blaze heats around each of its flames, not only its middle: what it may break is listed
  -- now and then (a wide look around its middle), then each tick the nearest flame counts.
  for _, b in ipairs(blazes) do
    if now - (b.heat_list_at or -100) > 4 then
      b.heat_list_at = now
      b.heat_list = {}
      for _, a in ipairs(actors_near(b.at, F.blaze_reach + H.blaze_reach + 300)) do
        if heatable(a) then b.heat_list[#b.heat_list + 1] = a end
      end
    end
    local rate = H.blaze_rate * (b.stage == 2 and 1.5 or 1)
    for _, a in ipairs(b.heat_list) do
      if a.alive ~= false and heatable(a) then
        local l = a:get("Location")
        local best = distance(l, b.at)
        for _, sp in ipairs(b.spot_points or {}) do
          local d = distance(l, sp.at)
          if d < best then best = d end
        end
        heat_one(a, l, best, H.blaze_reach, rate, b.by, dt)
      end
    end
  end
end

-- Who stands in the fires left on the scenery: characters catch fire, players get hurt.
local function fire_contact(fx, by, reach, now, dt, people, inside)
  if not fx.alive then return end
  local at = fx:get("Location")
  for _, c in ipairs(characters_near(at, reach + 40)) do
    if c.alive then ignite(c, by, now) end
  end
  for _, p in ipairs(people) do
    if p.alive then
      local name = p.name
      if not inside[name] then
        local q = p:get("Location")
        q[3] = q[3] - p:get("CollisionHeight")
        if distance(q, at) < reach + p:get("CollisionRadius") then
          inside[name] = true
          scorch_player(p, by, dt)
        end
      end
    end
  end
end

local function fires_hurt(now, dt)
  if #fires == 0 and #blazes == 0 then return end
  local inside = {}
  local people = players()
  for _, f in ipairs(fires) do
    fire_contact(f.fx, f.by, F.ground_fire_reach, now, dt, people, inside)
    for _, t in ipairs(f.tongues) do
      fire_contact(t, f.by, F.ground_fire_reach, now, dt, people, inside)
    end
  end
  for _, b in ipairs(blazes) do
    for _, fx in ipairs(b.spots) do
      fire_contact(fx, b.by, F.blaze_damage_reach, now, dt, people, inside)
    end
  end
end

-- A body carried on a player's shoulder is drawn by another actor (the left hand's HeldBody):
-- it gets the charred look of the body it shows.
local function carried_bodies()
  for _, p in ipairs(players()) do
    local ok = pcall(function()
      local hand = p:get("LHand")
      if hand == nil then return end
      local body = hand:get("pOnShoulder")
      local held = hand:get("TheHeldBody")
      if body ~= nil and held ~= nil and body.charred > 0 then held.charred = body.charred end
    end)
  end
end

-- Fires left on the scenery die out: no new flames, then gone.
local function fires_out(now)
  for i = #fires, 1, -1 do
    local f = fires[i]
    if f.crackle and f.crackle.alive and now <= f.until_t and now - f.sound_at > F.sound_every * 1.5 then
      f.sound_at = now
      fire_sounds_again(f.fx, f.crackle, F.fire_sounds)
    end
    if now > f.until_t then
      for _, fx in ipairs({f.fx, table.unpack(f.tongues or {})}) do
        if fx.alive then
          fx:trigger(f.by)
          fx:set("LifeSpan", 3)
        end
      end
      silence(f.fx, f.crackle)
      if f.smoke and f.smoke.alive then
        f.smoke:trigger(f.by)
        f.smoke:set("LifeSpan", 4)
      end
      table.remove(fires, i)
    end
  end
end

-- The tanks on the back of a player who holds the flamethrower (taken off otherwise).
local function wear_tanks(p, s, held)
  if held and (s.tanks == nil or not s.tanks.alive) and F.tanks then
    local t = effect{class = F.tanks.class, position = p:get("Location")}
    if t then
      pcall(function()
        t:call("SetOwner", p)
        t:call("SetCollision", false, false, false)
        p:call("AttachToBone", t, F.tanks.bone)
        t:call("SetRelativeLocation", F.tanks.offset)
        t:call("SetRelativeRotation", F.tanks.rotation)
      end)
      s.tanks = t
    end
  elseif not held and s.tanks ~= nil then
    if s.tanks.alive then remove(s.tanks) end
    s.tanks = nil
  end
end

-- The muzzle of the lance seen in a character's hands, without its third-person actor: in
-- front of his eyes, where he aims.
local function muzzle_third(v)
  local m = F.muzzle_third
  return add(add(add(v.eyes, v.fwd, m[1]), v.right, m[2]), v.up, m[3])
end

-- The gun's actor seen in a character's hands (nil while there is none).
local function third_person(p)
  local a = nil
  pcall(function() a = p:get("Weapon"):get("ThirdPersonActor") end)
  if a ~= nil and a.alive then return a end
  return nil
end

-- A rotation (game units) pointing along d.
local function rotation_along(d)
  local pitch = math.atan(d[3], math.sqrt(d[1] * d[1] + d[2] * d[2]))
  local yaw = math.atan(d[2], d[1])
  return {pitch = math.floor(pitch * 32768 / math.pi) % 65536, yaw = math.floor(yaw * 32768 / math.pi) % 65536, roll = 0}
end

-- The shots of each player's gun (on the host, a network guest's too: the host fires it there
-- shot after shot).
local shot_at = {}
on("fire", function(p) shot_at[p.name] = time() end)

local function step(p, now, dt)
  local s = players_state[p.name]
  if s == nil then s = {} players_state[p.name] = s end
  local held = p.alive and p.weapon == F.weapon
  -- An object in hand (a medikit, the grapple, a key...): the gun is put down.
  if held then
    pcall(function()
      local c = p:get("Controller")
      if c and c:get("bWeaponMode") == false then held = false end
    end)
  end
  wear_tanks(p, s, held)
  if not held then
    if s.hiss then
      for _, h in ipairs(s.hiss) do silence(nil, h) end
      s.hiss = nil
    end
    for _, j in ipairs({s.jet or false, s.jet3 or false}) do
      if j and j.alive then j:set("Emitters[0].Disabled", true) j:set("Emitters[1].Disabled", true) end
    end
    for _, pf in ipairs({s.pilot or false, s.pilot3 or false}) do
      if pf and pf.alive then pf:set("Emitters[0].Disabled", true) end
    end
    return
  end
  local v = view(p)
  if v == nil then return end
  local nozzle = add(add(add(v.eyes, v.fwd, F.nozzle[1]), v.right, F.nozzle[2]), v.up, F.nozzle[3])
  -- Where the player sees his own flame come out: the tip of the lance drawn in his hands (the
  -- weapon's muzzle field), or the nozzle in front of the eyes when it is not drawn here.
  local tip = nil
  pcall(function() tip = p:get("Weapon").muzzle end)
  local own = tip or nozzle
  -- Two jets: the one the player sees, from the lance in his hands in first person (only he
  -- sees it), and the one the others see, from the muzzle of the lance in his hands.
  local muzzle = muzzle_third(v)
  -- Alone, nobody else looks: one jet only.
  local others = #players() > 1
  if s.jet == nil or not s.jet.alive or s.pilot == nil or not s.pilot.alive
      or (others and (s.jet3 == nil or not s.jet3.alive or s.pilot3 == nil or not s.pilot3.alive)) then
    for _, old in ipairs({s.jet or false, s.pilot or false, s.jet3 or false, s.pilot3 or false}) do
      if old and old.alive then remove(old) end
    end
    s.jet = effect{class = "Mod.FlameJet", position = own, rotation = {0, 0, 0}}
    s.pilot = effect{class = "Mod.PilotFlame", position = own, rotation = {0, 0, 0}}
    s.jet3, s.pilot3, s.pinned = nil, nil, nil
    if others then
      s.jet3 = effect{class = "Mod.FlameJet", position = muzzle, rotation = {0, 0, 0}}
      s.pilot3 = effect{class = "Mod.PilotFlame", position = muzzle, rotation = {0, 0, 0}}
    end
    for _, mine in ipairs({s.jet or false, s.pilot or false}) do
      if mine then pcall(function() mine:call("SetOwner", p) mine:set("bOnlyOwnerSee", true) end) end
    end
    for _, theirs in ipairs({s.jet3 or false, s.pilot3 or false}) do
      if theirs then pcall(function() theirs:call("SetOwner", p) theirs:set("bOwnerNoSee", true) end) end
    end
  end
  local moved = {s.jet, s.pilot}
  for _, h in ipairs(s.hiss or {}) do moved[#moved + 1] = h end
  for _, fx in pairs(moved) do
    if fx and fx.alive then
      fx:call("SetLocation", own)
      fx:call("SetRotation", v.rotation)
    end
  end
  -- The others' flame: pinned at the tip of the lance in his hands, turned towards what he aims at.
  local hands = third_person(p)
  for _, fx in ipairs({s.jet3 or false, s.pilot3 or false}) do
    if fx and fx.alive then
      if hands ~= nil then
        if s.pinned ~= hands.name then fx:pin_to(hands, F.muzzle_tip) end
        local tip = fx:get("Location")
        fx:call("SetRotation", rotation_along(sub(add(v.eyes, v.fwd, F.aim_point), tip)))
      else
        fx:pin_to(nil)
        fx:call("SetLocation", muzzle)
        fx:call("SetRotation", v.rotation)
      end
    end
  end
  s.pinned = hands and hands.name
  -- The fire button: a player of this computer holds it (bFire). A network guest's gun is fired
  -- on the host shot after shot (its fuel goes down, about once a second: the firing animation
  -- is the long waiting one), and the guest's game says when the button is let go (the
  -- weapon's StopFiringTime changes): the flame burns from a shot until then.
  local left = fuel(p)
  local now = time()
  local stop = 0
  pcall(function() stop = p:get("Weapon"):get("StopFiringTime") end)
  if s.stop_seen ~= nil and stop ~= s.stop_seen then s.stopped_at = now end
  s.stop_seen = stop
  local shot = shot_at[p.name]
  local guest_firing = shot ~= nil and now - shot < 1.6 and (s.stopped_at == nil or s.stopped_at < shot)
  local firing = left > 0 and (v.controller:get("bFire") ~= 0 or guest_firing)
  for _, pf in ipairs({s.pilot or false, s.pilot3 or false}) do
    if pf and pf.alive then pf:set("Emitters[0].Disabled", false) end
  end
  for _, j in ipairs({s.jet or false, s.jet3 or false}) do
    if j and j.alive then
      j:set("Emitters[0].Disabled", not firing)
      j:set("Emitters[1].Disabled", not firing)
    end
  end
  -- The jet's sound: its loop starts with the flame, from a silent emitter at the nozzle, and
  -- stops with it (the emitter is taken out of the game).
  if firing and s.hiss == nil then
    if now - (s.burst_at or -1) > F.burst_gap then
      for _, b in ipairs(F.burst_sounds) do play_once(b[1], b[2], nozzle, b[3]) end
    end
    s.burst_at = now
    s.hiss = {}
    for _, name in ipairs(F.jet_sounds) do
      s.hiss = sound_layers(name, F.jet_loudness, nozzle)
      if #s.hiss > 0 then break end
    end
  elseif not firing and s.hiss ~= nil then
    for _, h in ipairs(s.hiss) do silence(nil, h) end
    s.hiss = nil
  end
  if firing and F.heat_strength > 0 then
    pcall(heat, {at = add(nozzle, v.fwd, F.range * 0.45), radius = F.range * 0.4, seconds = 0.15, strength = F.heat_strength, name = "jet " .. p.name})
  end
  if firing then
    sweep(p, v, nozzle, now)
    scorch(p, v, nozzle, now)
  end
  s.firing = firing
end

-- One part that fails (an actor gone at the wrong moment) must never stop the others: each is
-- run on its own, and its error written once in mods-log.txt.
local reported = {}
local function guarded(what, f, ...)
  local ok, why = pcall(f, ...)
  if not ok and not reported[what .. tostring(why)] then
    reported[what .. tostring(why)] = true
    log("flamethrower: " .. what .. ": " .. tostring(why))
  end
end

-- The weapon in hand at the start of a level, for each player once he can move.
local given = {}
local function give_weapons()
  if not F.give_at_start or map():lower():find("menu") ~= nil or not has_control() then return end
  for _, p in ipairs(players()) do
    if p.alive and not given[p.name] then
      given[p.name] = true
      p:give(F.weapon)
      pcall(function() p:get("Controller"):call("SwitchWeapon", 18) end)
      after(0.3, function()
        if (p.ammo or F.start_fuel) < F.start_fuel then p.ammo = F.start_fuel end
      end)
    end
  end
end

-- The refill key: with the flamethrower in hand, its fuel is filled up.
if F.refill_key then
  on("key", F.refill_key, function(p)
    if map():lower():find("menu") ~= nil or p.weapon ~= F.weapon then return end
    pcall(function() p.ammo = 600 end)
  end)
end

on("start", function()
  local last = time()
  while true do
    guarded("weapon", give_weapons)
    local now = time()
    local dt = now - last
    last = now
    for _, p in ipairs(players()) do guarded("player", step, p, now, dt) end
    guarded("burning", burn, now, dt)
    guarded("fires", fires_out, now)
    guarded("ground fires", fires_hurt, now, dt)
    guarded("blazes", blazes_step, now, dt)
    guarded("heat", heat_props, now, dt)
    guarded("carried bodies", carried_bodies)
    wait(0)
  end
end)
