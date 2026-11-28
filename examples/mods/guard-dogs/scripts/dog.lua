-- Guard dogs: what a dog does.
--
-- The game has a dog nobody ever placed in a level: a model, fifteen animations, thirty-three
-- recordings and a "bite" damage type, but no AI. This file is that AI, written with the
-- script functions only. places.lua says where the dogs are; it calls Dog.place{...} defined
-- here (a mod's scripts share their global names, and run in the order of their file names).
--
-- A dog is in one mood at a time:
--
--   heel     follows its master (or keeps its place): walks behind him, sits when he stops,
--            lies down when he stays
--   alert    has smelled a player: gets up, turns to the smell, growls
--   search   walks to the smell with its nose on the ground
--   chase    has seen the player: barks (the soldiers around hear it) and runs at him
--   bite     leaps at him, lands in front of him and bites; leaps again when he steps back
--   retreat  badly hurt (once): runs out of the fight, then comes back from another side
--   grieve   its master was killed without anyone noticing: goes round him, nose down, crying
--   hunt     then looks for whoever did it, nose down, where its master used to walk; finds
--            nobody: goes back to lie by him (heel, sad)
--
-- Every animation of the model and every recording of the dog has its moment; the tables
-- below say which.

Dog = {}

-- ---------------------------------------------------------------------------------------
-- Settings. Distances are in the game's units (a man is about 160 high, a dog 150 long).

local SMELL = 1100          -- a dog smells a player this far away, walls or not
local SMELL_HEIGHT = 300    -- but not from another floor
local KEEN = 1.5            -- a dog that hunts smells this many times further
local SIGHT = 2200          -- and sees him this far away, with its own eyes
local LOST = 2800           -- further than this, the chase is over
local LEAP = 520            -- a leap starts from here at most
local LEAP_MIN = 300        -- and from here at least (closer, it just bites)
local REACH = 150           -- it lands, and stands to bite, this far from the player
local BITE = 175            -- the teeth reach this far (from the dog's middle to the player's)
local BITE_DAMAGE = 40      -- a bite, before the game makes it smaller with the difficulty (as for bullets)
local HEEL = 190            -- a dog at heel stays this close to its master
local PACK = 1500           -- a bark alerts the soldiers this far around
local RETREAT = 900         -- how far it runs when it has had enough
local GRIEF = 7.5           -- seconds spent going round its dead master
local HUNT = 25             -- seconds spent looking for the killer

-- Gaits: the animation played while the dog moves, the speed the dog moves at (units per
-- second) and the animation's own speed: how fast the ground goes by under the paws when
-- the animation plays at its normal rate (measured on the model). The script plays the
-- animation at (the dog's real speed) / stride, so the paws never slide, at any speed.
local GAITS = {
  walk  = {animation = "Walk",       speed = 190, stride = 177, cycle = 21 / 30},
  sniff = {animation = "WalkSearch", speed = 150, stride = 177, cycle = 21 / 30},
  run   = {animation = "Run",        speed = 640, stride = 660, cycle = 13 / 30},
}
local LEAP_SPEED = 700            -- the speed of the leap
local LEAP_HEIGHT = 55            -- and how high it goes
local LEAP_EVERY = 2.5            -- seconds between two leaps, at least
local LEAP_ANIMATION = "Jump"     -- running, leaves the ground and lands (0.7 s)

-- The animations played on the spot, and how long they last (frames / 30).
local SIT = "assie"               -- sitting (0.4 s, looped)
local SIT_LONG = "assie200"       -- sitting, looking around (6.7 s)
local LIE = "couche"              -- lying, head up (3.4 s)
local LIE_RELAXED = "coucherelax" -- lying, at rest (3.4 s)
local LOOK_BACK = "Demitr"        -- lying, turns to look behind (0.5 s)
local TURN = "TourneG"            -- standing, starts to turn (0.3 s)
local STAMP = "PietineG"          -- standing, stamps on the spot (0.5 s, looped)
local ATTACK = "Attak"            -- crouched, snaps forward (0.7 s)
local ATTACK_END = "AttakBackNeu" -- from the attack back to standing (1.3 s)
local DEATH = "Death"             -- thrown back, falls on its side (2.4 s)
local DEATH_RUNNING = "death2"    -- shot while running: tumbles forward (1.7 s)

-- The recordings, by what they sound like, and how long the longest of each lasts.
local SOUND = {
  -- A dog somewhere out of sight.
  far     = {seconds = 1.5, "XIIISound.Ambient.DogAmbient1", "XIIISound.Ambient.DogAmbient2", "XIIISound.Ambient.DogAmbient3"},
  -- A warning growl: it wants to bite.
  threat  = {seconds = 1.2, "XIIISound.PNJ.DogGrowl2"},
  -- A bark, then a growl: it has found him.
  found   = {seconds = 1.0, "XIIISound.PNJ.DogGrowl1"},
  -- Angry barks.
  bark    = {seconds = 1.1, "XIIISound.PNJ.DogBark1", "XIIISound.PNJ.DogBark2", "XIIISound.PNJ.DogBark3"},
  -- Fighting barks (the third, the meanest, less often).
  leap    = {seconds = 0.6, "XIIISound.PNJ.DogLunge1", "XIIISound.PNJ.DogLunge2", "XIIISound.PNJ.DogLunge1", "XIIISound.PNJ.DogLunge2", "XIIISound.PNJ.DogLunge3"},
  -- The jaws.
  bite    = {seconds = 1.3, "XIIISound.PNJ.DogBit1", "XIIISound.PNJ.DogBit2", "XIIISound.PNJ.DogBit3"},
  -- Pain.
  hit     = {seconds = 0.6, "XIIISound.PNJ.DogDieA1"},
  -- A low, heavy growl: hurt, and it wants its own back.
  grudge  = {seconds = 1.2, "XIIISound.PNJ.DogGrowl3"},
  -- A wounded dog.
  wounded = {seconds = 0.6, "XIIISound.PNJ.DogDieB1"},
  -- Pain and sorrow.
  death   = {seconds = 0.9, "XIIISound.PNJ.DogDieA2"},
  -- A sad dog: these four are for its dead master.
  cry     = {seconds = 0.4, "XIIISound.PNJ.DogYelp2"},
  cry_on  = {seconds = 0.5, "XIIISound.PNJ.DogYelp3"},
  cry_low = {seconds = 0.6, "XIIISound.PNJ.DogYelp1"},
  moan    = {seconds = 1.0, "XIIISound.PNJ.DogDieB2"},
}

-- Paw steps, by the ground.
local STEPS = {
  grass  = {"XIIISound.Footsteps.FtDogHrb1", "XIIISound.Footsteps.FtDogHrb2", "XIIISound.Footsteps.FtDogHrb3", "XIIISound.Footsteps.FtDogHrb4"},
  earth  = {"XIIISound.Footsteps.FtDogHer1"},
  gravel = {"XIIISound.Footsteps.FtDogGra1"},
  snow   = {"XIIISound.Footsteps.FtDogNei1"},
  tiles  = {"XIIISound.Footsteps.FtDogCar1"},
  metal  = {"XIIISound.Footsteps.FtDogMet1"},
  water  = {"XIIISound.Footsteps.FtDogWat1", "XIIISound.Footsteps.FtDogWat2"},
}
-- The level says what each floor is for a man's footsteps (actor.ground: "PNJFSNei" on
-- snow...): the three letters after PNJFS give the dog's. Any other floor is a hard one.
local GROUNDS = {
  Hrb = "grass", Feu = "grass", Ter = "earth", Moq = "earth", Gra = "gravel", Nei = "snow",
  Met = "metal", Tol = "metal", Gri = "metal",
}

-- ---------------------------------------------------------------------------------------
-- Small tools.

local function distance(a, b)
  local p, q = a.position or a, b.position or b
  return math.sqrt((p[1] - q[1]) ^ 2 + (p[2] - q[2]) ^ 2 + (p[3] - q[3]) ^ 2)
end

local function pick(list)
  return list[random(#list)]
end

-- One of the recordings of a kind, from the dog. A dog has one voice: while one recording
-- plays, another waits its turn (false), unless it is `urgent` (pain cuts a bark short).
-- How far it is heard is the dog's own setting (mod.toml, StabilisationDistance).
local function voice(me, kind, urgent)
  if not urgent and me.clock < me.quiet then return false end
  me.quiet = me.clock + kind.seconds
  -- A sound that may be missing from an installation never stops a dog.
  pcall(sound, pick(kind), me.dog)
  return true
end

-- The player nearest to the dog, and how far he is.
local function nearest_player(dog)
  local best, best_distance
  for _, p in ipairs(players()) do
    if p.alive then
      local d = distance(dog, p)
      if not best or d < best_distance then best, best_distance = p, d end
    end
  end
  return best, best_distance or math.huge
end

-- The dog's controller: the game object that moves it (CineController2).
local function controller(dog)
  return dog:get("Controller")
end

-- Whether the dog sees the actor with its own eyes (walls hide him).
local function sees(dog, actor)
  local ok, seen = pcall(dog.can_see, dog, actor)
  return ok and seen
end

-- Whether a dog smells the player (`keen`: from further away).
local function smells(dog, p, keen)
  local a, b = dog.position, p.position
  return math.abs(a[3] - b[3]) < SMELL_HEIGHT and distance(a, b) < SMELL * (keen and KEEN or 1)
end

-- The angle the dog has to turn to face a point, in degrees (-180 to 180).
local function turn_to(dog, point)
  local at = dog.position
  local wanted = math.deg(math.atan(point[2] - at[2], point[1] - at[1]))
  return (wanted - dog.yaw + 180) % 360 - 180
end

-- Whether a soldier around is fighting a player.
local function fight_around(dog)
  for _, someone in ipairs(characters_near(dog, PACK)) do
    local ok, enemy = pcall(function() return someone:get("Controller"):get("Enemy") end)
    if ok and enemy then
      for _, p in ipairs(players()) do
        if enemy == p then return true end
      end
    end
  end
  return false
end

-- ---------------------------------------------------------------------------------------
-- Moving and standing. `me` is what the script knows about one dog (Dog.pack).
--
-- Cine2 has animations of its own for a man who walks; the script leaves them alone and
-- plays the dog's itself (animate, below): the gait while it moves, at the rate that fits
-- its real speed; the idle animation while it stands; and now and then one animation played
-- once (a leap, a bite).

-- Sets the gait and the speed the dog moves at.
local function gait(me, name, speed)
  local c = controller(me.dog)
  me.gait = name
  -- SetWantedSpeed takes a part of the speed the controller read when it was set up.
  c:call("SetWantedSpeed", (speed or GAITS[name].speed) / c:get("MyPawnGroundSpeed"))
end

-- Whether the dog is on its way somewhere.
local function moving(dog)
  return controller(dog):get("bMoving")
end

-- Stops where it is. `idle` is the animation it takes when standing there (looped).
local function halt(me, idle)
  if idle then me.idle = idle end
  if moving(me.dog) then me.dog:stop() end
end

-- Plays an animation once, whatever the dog is doing, for its `seconds`.
local function once(me, animation, seconds, rate)
  controller(me.dog):call("PlayAni", animation, rate or 1.0, 0.1)
  me.busy = seconds / (rate or 1.0)
end

-- What the paws walk on: water, what the level says of the floor, or what places.lua said.
local function ground(me)
  local dog = me.dog
  local ok, wet = pcall(function() return dog:get("PhysicsVolume"):get("bWaterVolume") end)
  if ok and wet then return "water" end
  local kind = (dog.ground or ""):match("PNJFS(%a%a%a)")
  if kind then return GROUNDS[kind] or "tiles" end
  return me.ground
end

-- Ten times a second: the animation that fits what the dog is doing, and its paw steps.
local function animate(me, dt)
  local dog = me.dog
  local c = controller(dog)
  if not c then return end   -- dying: the game has taken its controller away
  me.clock = me.clock + dt
  if me.busy then
    me.busy = me.busy - dt
    if me.busy > 0 then return end
    me.busy = nil
  end
  if moving(dog) then
    local g = GAITS[me.gait]
    local v = dog:get("Velocity")
    local speed = math.sqrt(v[1] ^ 2 + v[2] ^ 2)
    -- How steep the ground is where it runs (for the way it dies, below): how much it
    -- went up or down since the last look, for how far it went.
    local at = dog.position
    if me.was then
      local along = math.sqrt((at[1] - me.was[1]) ^ 2 + (at[2] - me.was[2]) ^ 2)
      if along > 5 then me.steep = math.abs(at[3] - me.was[3]) / along end
    end
    me.was = at
    local rate = math.max(speed / g.stride, 0.25)
    -- Again only when the animation or its rate changes (the game keeps a loop going), or
    -- when a new way has given the animations back to Cine2 (bCineControlAnims).
    if c:get("CurrentAnim") ~= g.animation or math.abs(rate - (me.rate or 0)) > 0.06 or not c:get("bCineControlAnims") then
      c:call("LoopAni", g.animation, rate, 0.15)
      me.rate = rate
    end
    -- Paw steps: two a stride.
    me.step = (me.step or 0) + dt * rate
    if me.step >= g.cycle / 2 then
      me.step = 0
      -- From where the dog stands, not from the dog: its voice is heard from far (mod.toml),
      -- its steps no further than a man's.
      pcall(sound, pick(STEPS[ground(me)] or STEPS.earth), dog.position)
    end
  elseif c:get("CurrentAnim") ~= me.idle or not c:get("bCineControlAnims") then
    c:call("LoopAni", me.idle, 1.0, 0.25)
    me.rate = nil
    me.was, me.steep = nil, nil
  end
end

-- Goes to a point and waits until it is there, cannot get further, or `seconds` have gone
-- by. `watch`, called four times a second, can cut the way short by giving a value: that
-- value is returned.
local function travel(me, point, seconds, watch)
  local dog = me.dog
  if not pcall(function() dog:go_to(point) end) then return nil end
  local last, stuck = dog.position, 0
  for _ = 1, seconds * 4 do
    wait(0.25)
    if watch then
      local cut = watch()
      if cut then return cut end
    end
    local at = dog.position
    if distance(at, point) < 110 or not moving(dog) then break end
    -- Against a wall: it runs and gets nowhere.
    stuck = distance(at, last) < 15 and stuck + 1 or 0
    if stuck >= 3 then break end
    last = at
  end
  return nil
end

-- Called again and again while the dog is on its way to `goal` (`dt` seconds since the
-- last call): when it runs and gets nowhere (a rock, a crate, a corner the level's paths do
-- not know), it steps aside, one side then the other, and takes its way again.
local function unstick(me, goal, dt)
  local dog = me.dog
  if not moving(dog) then
    me.stuck_at = nil
    return false
  end
  local at = dog.position
  -- "Nowhere": a fifth of what its speed should have covered.
  if me.stuck_at and distance(at, me.stuck_at) < math.max(dog:get("GroundSpeed") * 0.12, 8) then
    me.stuck_for = me.stuck_for + dt
  else
    me.stuck_at, me.stuck_for = at, 0
  end
  if me.stuck_for < 0.6 then return false end
  me.stuck_at = nil
  me.side = -(me.side or 1)
  local to = goal.position or goal
  local dx, dy = to[1] - at[1], to[2] - at[2]
  local far = math.max(math.sqrt(dx * dx + dy * dy), 1)
  dx, dy = dx / far, dy / far
  -- Aside and a little back.
  pcall(function() dog:go_to({at[1] - dy * 260 * me.side - dx * 80, at[2] + dx * 260 * me.side - dy * 80, at[3]}) end)
  wait(0.7)
  pcall(function() dog:go_to(goal) end)
  return true
end

-- Whether the dog's master is fighting this player: then the dog fights him too, whatever
-- it sees or smells itself.
local function master_fights(me, p)
  local master = me.master
  if not master or not master.alive then return false end
  local ok, enemy = pcall(function() return master:get("Controller"):get("Enemy") end)
  return ok and enemy ~= nil and enemy == p
end

-- ---------------------------------------------------------------------------------------
-- The moods. Each one is a function that runs until the dog changes its mind, and gives
-- the next mood.

local moods = {}

-- Whether it has just lost health; it cries out when it has.
local function hurt(me)
  if me.dog.health >= me.health then return false end
  me.health = me.dog.health
  voice(me, SOUND.hit, true)
  me.grudge = true
  return true
end

-- Whether it has had enough: half its health gone, for the first time.
local function beaten(me)
  return not me.retreated and me.dog.health <= me.full / 2
end

-- Whether its master has just been killed with nobody the wiser: no soldier around is
-- fighting a player (the dog itself may have smelled something; it has not seen him).
local function bereaved(me)
  local master = me.master
  return master ~= nil and not master.alive and not fight_around(me.dog)
end

-- What every calm mood checks first: a player to go for? Gives the mood to take.
local function notice(me)
  local dog = me.dog
  local p, d = nearest_player(dog)
  if not p then return nil end
  me.target = p
  -- Hurt: it knows where that came from.
  if dog.health < me.health then return "chase" end
  -- Its master is fighting a player: so does the dog.
  for _, someone in ipairs(players()) do
    if someone.alive and master_fights(me, someone) then
      me.target = someone
      return "chase"
    end
  end
  -- Set on a player by the level (Dog.sic).
  if me.sic then
    local target = me.sic
    me.sic = nil
    if target.alive then
      me.target = target
      return "chase"
    end
  end
  -- On a leash: it waits for one of the reasons above.
  if me.leash then return nil end
  if d < SIGHT and sees(dog, p) and math.abs(turn_to(dog, p.position)) < 100 then return "chase" end
  if smells(dog, p, me.keen) then return "alert" end
  return nil
end

-- At heel: behind its master, or at its own place when it has none (a dog whose master is
-- dead keeps his place, and is sad).
function moods.heel(me)
  local dog = me.dog
  local still = 0          -- seconds spent waiting on the spot
  local down = false       -- lying
  local sound_in = random(5, 9)
  gait(me, "walk")
  while true do
    local mood = notice(me)
    if mood then
      if down then once(me, LOOK_BACK, 0.5) wait(0.5) end
      return mood
    end
    local master = me.master
    local pace = 0           -- how fast its master walks
    -- A dog placed without anybody around takes the first character who comes by.
    if me.adopt and not master then
      master = Dog.nearest_master(dog.position, 800)
      me.master = master
      if master then me.adopt = nil end
    end
    if master then
      if not master.alive then
        -- Killed. With a fight going on around, the dog joins it; with nobody the wiser,
        -- it does not know what happened.
        if fight_around(dog) then return "chase" end
        return "grieve"
      end
      -- It remembers where he is (some levels take a body away at once), and his round.
      local at = master.position
      if me.master_at then pace = distance(at, me.master_at) / 0.25 end
      me.master_at = at
      local rounds = me.rounds
      if #rounds == 0 or distance(rounds[#rounds], at) > 300 then
        rounds[#rounds + 1] = at
        if #rounds > 8 then table.remove(rounds, 1) end
      end
    end
    -- A dog told to lie at its place lets its master walk his round.
    local goal = not me.stay and master or me.home
    if me.stay then pace = 0 end
    local d = distance(dog, goal)
    if d > 700 then
      -- Far behind: runs to catch up.
      gait(me, "run")
      if not moving(dog) then dog:go_to(goal) end
      unstick(me, goal, 0.25)
      still, down = 0, false
    elseif pace > 30 or d > HEEL * 1.6 then
      -- Walks at its master's pace: faster when it falls behind, slower when it is on his
      -- heels, so that it stays a few steps behind him.
      local keep = math.max(0.3, math.min(1.4, 1 + (d - HEEL) / 120))
      if pace > 30 then gait(me, "walk", math.max(pace * keep, 40)) else gait(me, "walk") end
      if not moving(dog) then dog:go_to(goal) end
      unstick(me, goal, 0.25)
      still, down = 0, false
    else
      -- Its master stands: it sits and watches him.
      if moving(dog) or still == 0 then
        halt(me, me.stay and LIE or SIT)
        if master and not me.stay then dog:look_at(master) end
      end
      still = still + 0.25
      -- Sits, looks around after a while, then lies down, then rests. A sad dog lies down
      -- at once.
      if me.sad or me.stay then
        if still == 1 then me.idle, down = LIE, true end
        if still == 12 and me.stay then me.idle = LIE_RELAXED end
      elseif still == 4 then
        me.idle = SIT_LONG
      elseif still == 18 then
        me.idle, down = LIE, true
      elseif still == 40 then
        me.idle = LIE_RELAXED
      end
    end
    -- Now and then: a sad dog moans; another is heard from afar when no player sees it.
    sound_in = sound_in - 0.25
    if sound_in <= 0 then
      sound_in = random(6, 11)
      if me.sad then
        voice(me, random(2) == 1 and SOUND.moan or SOUND.cry)
      else
        local p = nearest_player(dog)
        if p and not sees(dog, p) then voice(me, SOUND.far) end
      end
    end
    wait(0.25)
  end
end

-- Alert: something in the air. Gets up, turns to it and growls.
function moods.alert(me)
  local dog, p = me.dog, me.target
  halt(me, STAMP)
  voice(me, SOUND.threat, true)
  if math.abs(turn_to(dog, p.position)) > 25 then
    once(me, TURN, 0.27)
    dog:look_at(p)
    wait(0.3)
  end
  -- A second or so to make up its mind (a shot ends it at once): in sight, it goes for
  -- him; hidden, it follows its nose.
  wait_until(function() return not p.alive or dog.health < me.health end, 1.2)
  if not p.alive then return "heel" end
  if dog.health >= me.health and bereaved(me) then return "grieve" end
  if dog.health < me.health or (distance(dog, p) < SIGHT and sees(dog, p)) then return "chase" end
  return "search"
end

-- Search: nose down, to where the smell comes from. The smell is where the player was a
-- moment ago: the dog follows his trail, not him.
function moods.search(me)
  local dog, p = me.dog, me.target
  local lost = 0
  local trail
  gait(me, "sniff")
  me.idle = STAMP
  while p.alive do
    local d = distance(dog, p)
    -- Found: hurt, nose to nose, or in sight (a dog that sniffs looks at the ground: it
    -- sees what is in front of it, or close).
    if dog.health < me.health or d < 200 or (d < SIGHT and sees(dog, p) and (d < 500 or math.abs(turn_to(dog, p.position)) < 100)) then
      return "chase"
    end
    -- Its master killed while it had its nose on a trail: the trail can wait.
    if bereaved(me) then return "grieve" end
    if smells(dog, p, me.keen) then
      lost = 0
      if not trail or distance(trail, p.position) > 150 or not moving(dog) then
        trail = p.position
        dog:go_to(trail)
      end
      unstick(me, trail, 0.5)
    else
      -- The smell is gone: goes to where it was last, sniffs around, then gives up.
      lost = lost + 0.5
      if lost > 6 then break end
    end
    if random(10) == 1 then voice(me, SOUND.threat) end
    wait(0.5)
  end
  return "heel"
end

-- The soldiers around hear a bark and come for the player too.
local function call_the_pack(dog, p)
  for _, someone in ipairs(characters_near(dog, PACK)) do
    -- attack() is for soldiers: story characters and other dogs refuse it, which is fine.
    pcall(function() someone:attack(p) end)
  end
end

-- A leap at the player: off the ground, and down in front of him.
local function leap(me)
  local dog, p = me.dog, me.target
  -- The jump animation is played so that it ends when the dog lands.
  local flight = math.max((distance(dog, p) - REACH) / LEAP_SPEED, 0.45)
  voice(me, SOUND.leap, true)
  gait(me, "run", LEAP_SPEED)
  once(me, LEAP_ANIMATION, 0.7, math.min(0.7 / flight, 2))
  dog:go_to(p)
  -- The animation stretches the dog out but keeps it on the ground: the script lifts the
  -- model along an arc (PrePivot moves a model without moving its actor).
  -- (Timed with the level's clock: a frame is not the same length on every PC.)
  local level = dog:get("Level")
  local started = level:get("TimeSeconds")
  wait_until(function()
    local part = math.min((level:get("TimeSeconds") - started) / flight, 1)
    dog:set("PrePivot", {X = 0, Y = 0, Z = 4 * LEAP_HEIGHT * part * (1 - part)})
    return part >= 1 or distance(dog, p) < REACH
  end, flight + 0.2)
  dog:set("PrePivot", {X = 0, Y = 0, Z = 0})
  halt(me, STAMP)
  gait(me, "run")
  me.leapt = me.clock
end

-- Whether there is room and time for a leap: far enough, in sight, and not straight after
-- the last one.
local function can_leap(me, d)
  return d > LEAP_MIN and d < LEAP and me.clock - (me.leapt or -10) > LEAP_EVERY and sees(me.dog, me.target)
end

-- Chase: runs at the player, barking.
function moods.chase(me)
  local dog, p = me.dog, me.target
  local unseen = 0
  me.leash = nil
  -- How it starts: a cry if it was shot, the heavy growl if it comes back for more, else
  -- the bark of a dog that has found him. The soldiers around hear it.
  if not hurt(me) then voice(me, me.back and SOUND.grudge or SOUND.found, true) end
  me.back = nil
  call_the_pack(dog, p)
  gait(me, "run")
  me.idle = STAMP
  -- A way round first (places.lua's `flank`, or its own after a retreat): once.
  if me.flank then
    for _, point in ipairs(me.flank) do
      travel(me, point, 4, function() hurt(me) end)
    end
    me.flank = nil
  end
  while p.alive do
    local d = distance(dog, p)
    if hurt(me) and beaten(me) then return "retreat" end
    local seen = sees(dog, p)
    if d < LEAP and seen then
      if can_leap(me, d) then leap(me) end
      return "bite"
    end
    -- Out of sight for long, or far away: the chase is over, unless its master still
    -- fights him (the dog does what its master does).
    if seen then unseen = 0 else unseen = unseen + 0.2 end
    if (d > LOST or unseen > 8) and not master_fights(me, p) then
      return smells(dog, p, me.keen) and "search" or "heel"
    end
    if not moving(dog) then dog:go_to(p) end
    unstick(me, p, 0.2)
    -- Barks all the way; hurt a moment ago, the growl of a dog that wants its own back.
    if me.grudge then
      if voice(me, SOUND.grudge) then me.grudge = nil end
    else
      voice(me, SOUND.bark)
    end
    wait(0.2)
  end
  return "heel"
end

-- Bite: bites for as long as the player stays within reach; when he steps back, leaps at
-- him again if there is room for a leap, else runs up to him.
function moods.bite(me)
  local dog, p = me.dog, me.target
  me.idle = STAMP
  while p.alive do
    local d = distance(dog, p)
    if hurt(me) and beaten(me) then return "retreat" end
    if d > LEAP then
      halt(me, STAMP)
      once(me, ATTACK_END, 1.3, 2)
      return "chase"
    elseif d > BITE then
      if can_leap(me, d) then
        leap(me)
      else
        gait(me, "run")
        if not moving(dog) then dog:go_to(p) end
        wait_until(function() return distance(dog, p) < REACH end, 0.2)
        if distance(dog, p) < REACH then halt(me, STAMP) end
      end
    else
      -- The bite: crouches and snaps forward; the jaws close a third of a second in.
      halt(me, STAMP)
      dog:look_at(p)
      once(me, ATTACK, 0.7)
      wait(0.3)
      if p.alive and distance(dog, p) < BITE * 1.1 then
        voice(me, SOUND.bite, true)
        p:hurt(BITE_DAMAGE, dog, "XIII.DTBite")
      end
      -- The rest of the animation, and a breath: the growl of a dog that was hit, if it was.
      wait(0.4)
      if me.grudge and voice(me, SOUND.grudge) then me.grudge = nil end
      wait(0.5)
    end
  end
  -- The player is down: back on its legs.
  halt(me, STAMP)
  once(me, ATTACK_END, 1.3)
  wait(1.3)
  return "heel"
end

-- Retreat: it has had enough. Runs out of the fight (to its master if he is alive and out
-- of it, else straight away from the player), gets its breath back, and comes back from
-- another side. Once in a dog's life.
function moods.retreat(me)
  local dog, p = me.dog, me.target
  me.retreated = true
  voice(me, SOUND.wounded, true)
  gait(me, "run")
  me.idle = STAMP
  local at, from = dog.position, p.position
  local dx, dy = at[1] - from[1], at[2] - from[2]
  local far = math.max(math.sqrt(dx * dx + dy * dy), 1)
  dx, dy = dx / far, dy / far
  local goal = {at[1] + dx * RETREAT, at[2] + dy * RETREAT, at[3]}
  local master = me.master
  if master and master.alive and distance(master, p) > 500 then goal = master.position end
  travel(me, goal, 4, function() hurt(me) end)
  halt(me, STAMP)
  wait(random(2, 4))
  if not p.alive then return "heel" end
  -- Back, by one side or the other of where the player is now.
  local side = random(2) == 1 and 1 or -1
  local to = p.position
  me.flank = {{to[1] + dx * 250 - dy * 450 * side, to[2] + dy * 250 + dx * 450 * side, to[3]}}
  me.back = true
  return "chase"
end

-- What a grieving or hunting dog still notices: a shot, or a player in front of it.
local function disturbed(me)
  local dog = me.dog
  local p, d = nearest_player(dog)
  if not p then return nil end
  me.target = p
  if dog.health < me.health then return "chase" end
  if d < SIGHT and sees(dog, p) and (d < 400 or math.abs(turn_to(dog, p.position)) < 100) then return "chase" end
  return nil
end

-- Grieve: its master is dead and nobody saw who did it. Goes to him nose down, goes round
-- him crying, sometimes lies down by him for a moment; then it wants to know.
function moods.grieve(me)
  local dog = me.dog
  local grave = me.master_at or me.home
  me.master = nil
  me.home = grave
  me.sad = true
  gait(me, "sniff")
  local watch = function() return disturbed(me) end
  local mood
  -- It stops and looks: two seconds to understand. Then the first cry, sometimes followed
  -- by another, and it goes to him nose down.
  halt(me, STAMP)
  pcall(function() dog:look_at(grave) end)
  for _ = 1, 8 do
    wait(0.25)
    mood = watch()
    if mood then return mood end
  end
  voice(me, SOUND.cry, true)
  if random(2) == 1 then
    wait(0.5)
    voice(me, SOUND.cry_on, true)
  end
  mood = travel(me, grave, 5, watch)
  if mood then return mood end
  -- Round him, a few steps at a time; a pause, sometimes a lower cry, then a long moan.
  gait(me, "sniff", 110)
  local at = dog.position
  local angle = math.atan(at[2] - grave[2], at[1] - grave[1])
  local started = me.clock
  local low, moaned = false, false
  while me.clock - started < GRIEF do
    angle = angle + 1.1
    mood = travel(me, {grave[1] + 130 * math.cos(angle), grave[2] + 130 * math.sin(angle), grave[3]}, 1.5, watch)
    if mood then return mood end
    local spent = me.clock - started
    if spent > 2.5 and not low then
      low = true
      if random(2) == 1 then voice(me, SOUND.cry_low) end
    elseif spent > 4.5 and not moaned then
      moaned = voice(me, SOUND.moan)
    end
  end
  -- Sometimes: lies down by him and cries, two seconds.
  if random(2) == 1 then
    halt(me, LIE)
    voice(me, SOUND.cry, true)
    for _ = 1, 8 do
      wait(0.25)
      mood = disturbed(me)
      if mood then return mood end
    end
  end
  return "hunt"
end

-- Hunt: looks for whoever did it. Nose down, back along its master's round, then around
-- the place, smelling further than usual. Finds a trail: follows it. Finds nothing: goes
-- back to lie by its master.
function moods.hunt(me)
  local dog = me.dog
  local grave = me.home
  me.keen = true
  gait(me, "sniff")
  me.idle = STAMP
  -- Where to look: the round, backwards, then three places around.
  local places = {}
  for i = #me.rounds, 1, -1 do places[#places + 1] = me.rounds[i] end
  for _ = 1, 3 do
    local angle, far = random() * 2 * math.pi, random(350, 700)
    places[#places + 1] = {grave[1] + far * math.cos(angle), grave[2] + far * math.sin(angle), grave[3]}
  end
  local started = me.clock
  local watch = function()
    if me.clock - started > HUNT then return "heel" end
    if random(24) == 1 then voice(me, SOUND.threat) end
    return notice(me)
  end
  for _, place in ipairs(places) do
    local mood = travel(me, place, 6, watch)
    if mood then return mood end
    -- A look around at each place.
    halt(me, STAMP)
    for _ = 1, 4 do
      wait(0.25)
      mood = watch()
      if mood then return mood end
    end
  end
  return "heel"
end

-- ---------------------------------------------------------------------------------------
-- A dog's life.

behaviour("GuardDog", function(self)
  local me = Dog.pack[self.name]
  -- The controller sets itself up during the dog's first moments.
  wait_until(function() return controller(self):get("bInitialized") end, 5)
  controller(self):call("SetRotationSpeed", 400)   -- a dog turns faster than a man
  controller(self):set("AccelerationFactor", 15)   -- and is at full speed in a few steps
  me.health = self.health
  me.full = self.health
  local mood = "heel"
  while self.alive and controller(self) do
    me.mood = mood
    -- The rare moods are written in Mods/mods-log.txt: what happened to a dog can be read
    -- there afterwards.
    if mood == "grieve" or mood == "hunt" or mood == "retreat" then
      log("guard dog " .. self.name .. ": " .. mood)
    end
    -- A dying dog loses its controller a moment before it is dead: whatever the mood was
    -- doing then stops there, without a word in the log.
    local ok, next = pcall(moods[mood], me)
    if not ok then
      if not self.alive or not controller(self) then return end
      -- Anything else: said in the log, and the dog goes on (a dog never stands frozen).
      log("guard dog " .. self.name .. " (" .. mood .. "): " .. tostring(next))
      wait(1)
      next = "heel"
    end
    mood = next or "heel"
    -- A frame between two moods: two moods that send the dog to each other never lock it.
    wait(0)
  end
end)

-- The animations and paw steps of every dog, ten times a second.
every(0.1, function()
  for _, me in pairs(Dog.pack) do
    if me.dog.alive and me.mood then animate(me, 0.1) end
  end
end)

Dog.pack = {}   -- the dogs of this map, by their actor's name

-- Dog.place{at = {x, y, z}, yaw = degrees, master = "who", ground = "snow", leash = true,
--           lying = true, flank = {points}}
--
--   at      where the dog starts (a point on the ground; `modpos` in the console gives
--           one); without it, next to its master
--   master  a character of the map the dog belongs to (as find() takes it, or the actor),
--           or "nearest": whoever is nearest to `at` when the dog comes (the names of the
--           characters change when a level is started from a checkpoint); without one the
--           dog guards its place
--   lying   true: the dog lies at its place while its master walks his round
--   ground  what the paws walk on where the level does not say: grass, earth, gravel, snow,
--           tiles, metal
--   leash   true: the dog does not go after a smell or a sight by itself; it waits until
--           it is hurt, its master fights, or the level sets it on a player (Dog.sic)
--   flank   points the dog runs through before it goes for the player, the first time
--
-- Returns the dog, or nil when there is no room there or the master is not in the level
-- (dead already at this checkpoint).
-- The character nearest to a point (not a dog, not a player), within `radius`.
function Dog.nearest_master(at, radius)
  for _, someone in ipairs(characters_near(at, radius)) do
    if not Dog.pack[someone.name] then return someone end
  end
  return nil
end

function Dog.place(t)
  local master
  if t.master == "nearest" then
    -- Whoever is nearest to the place now; nobody yet: the dog takes the first who comes.
    master = Dog.nearest_master(t.at, 900)
  elseif t.master then
    master = type(t.master) == "string" and find(t.master) or t.master
    if not master or not master.alive then return nil end
  end
  -- At the point given, or at the first free spot around its master.
  local dog
  if t.at then
    dog = spawn{character = "Dog", position = t.at, yaw = t.yaw or 0}
  else
    dog = spawn{character = "Dog", near = master, yaw = master.yaw}
  end
  if not dog then return nil end
  Dog.pack[dog.name] = {
    dog = dog, master = master, home = dog.position, ground = t.ground or "earth",
    leash = t.leash, flank = t.flank, stay = t.lying, adopt = t.master == "nearest" and not master,
    gait = "walk", idle = t.lying and LIE or SIT, clock = 0, quiet = 0, rounds = {}, stuck_for = 0,
  }
  dog:behave("GuardDog")
  -- Death: the behaviour has stopped with the dog; its last animation and its last sound.
  on("death", dog, function(dead)
    local me = Dog.pack[dead.name]
    -- Shot while it ran, it tumbles forward: that animation carries the body some way
    -- ahead, at the height it died at. On level ground only: down a slope the body would
    -- end in the air, up a slope in the ground.
    local running = (me.mood == "chase" or me.mood == "retreat") and (me.steep or 1) < 0.06
    pcall(function() dead:stop() end)   -- no way left to watch
    pcall(sound, pick(SOUND.death), dead)
    dead:call("PlayAnim", running and DEATH_RUNNING or DEATH, 1.0, 0.1)
    Dog.pack[dead.name] = nil
  end)
  return dog
end

-- Dog.sic(dog, player): sets a dog on a player, wherever he is (a level's scene).
function Dog.sic(dog, target)
  local me = dog and Dog.pack[dog.name]
  if me then me.sic = target end
end
