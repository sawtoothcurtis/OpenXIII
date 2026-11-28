# A whole enemy in Lua: guard dogs

`examples/mods/guard-dogs` adds an enemy the game never had: dogs that walk with the guards,
smell the player, hunt him down and bite. It is the largest example of the scripts, and a
model for an enemy of your own. Everything it shows comes from the player's own game: the mod
ships three text files and nothing else.

## What the game already holds

The game's files hold a dog that no level uses:

| What | Where | |
|---|---|---|
| The model | `XIIIPersos.DogM` | a German shepherd, 150 units long |
| Its animations | `XIIIPersos.DogA` | fifteen: walk, sniffing walk, run, leap, attack, sit, lie, two deaths... |
| Its voice | `XIIISound.PNJ.Dog...` | nineteen recordings: barks, growls, bites, cries |
| Dogs far away | `XIIISound.Ambient.DogAmbient1` to `3` | three recordings |
| Its paw steps | `XIIISound.Footsteps.FtDog...` | eleven recordings, by the kind of ground |
| A bite | `XIII.DTBite` | a damage type |

There is no dog AI in the game. The levels still carry traces of the plan: on the rooftops
(Toits01) the man who kicks a table over fires an event called `MortProprioChien`, "death of the
dog's owner".

## The three files

```text
guard-dogs/
  mod.toml              what a dog is: a new class with the dog's model
  scripts/dog.lua       what a dog does: its moods, its animations, its sounds
  scripts/places.lua    where the dogs are, level by level
```

`mod.toml` makes the dog a new class from the game's story characters (`XIDCine.Cine2`):
that class has no AI of its own and does what scripts tell it. A few values make it fit a dog:
its collision size, no man's voice, no man's footsteps. It is a class of its own, and not only
a `[character]` with other values, because the game goes back to a class's own size when a
character is hit, lands or dies: with a man's size, a dog would be pushed into the floor.

## A dog's moods

A dog is in one mood at a time; each mood is one function of `dog.lua` that runs until the dog
changes its mind and returns the name of the next one.

| Mood | What the dog does |
|---|---|
| `heel` | follows its master a few steps behind, at his pace; sits when he stops, lies down when he stays |
| `alert` | has smelled a player: gets up, turns to the smell, growls |
| `search` | walks to the smell with its nose on the ground: it follows the player's trail |
| `chase` | has seen the player: barks (the soldiers around hear it) and runs at him |
| `bite` | leaps at him, lands in front of him and bites; leaps again when he steps back |
| `retreat` | has lost half its health (once): runs out of the fight, then comes back from another side |
| `grieve` | its master was killed and nobody saw who did it (the dog may have smelled him; no soldier around fights him): two seconds later it cries, goes to him nose down and round him, sometimes lies down by him |
| `hunt` | then looks for whoever did it, back along its master's round; finding nobody, it goes back to lie by him |

Its senses: it smells a player through walls (but not from another floor), sees him with the
game's own line of sight, feels every shot, and knows when its master has a player for enemy.
With several players (split screen or network), it goes for the nearest one.

## What the script functions are used for

| To do this | The example uses |
|---|---|
| create a dog | `spawn{character = "Dog", ...}` |
| drive it for as long as it lives | `behaviour("GuardDog", fn)` and `dog:behave("GuardDog")` |
| move it | `dog:go_to(actor or point)`, `dog:stop()`, `dog:look_at(actor)` |
| set its speed | `controller:call("SetWantedSpeed", ...)` on `dog:get("Controller")` |
| play its animations | `controller:call("LoopAni", name, rate, blend)` and `"PlayAni"` |
| know what it walks on | `dog.ground` |
| see and find | `dog:can_see(player)`, `players()`, `characters_near(dog, radius)` |
| call the soldiers | `soldier:attack(player)` |
| bite | `player:hurt(amount, dog, "XIII.DTBite")` |
| play a recording | `sound("XIIISound.PNJ.DogBark1", dog)` |
| lift the model during a leap | `dog:set("PrePivot", {X = 0, Y = 0, Z = height})` |
| react to its death | `on("death", dog, fn)` |
| react to the level | `on("event", "SurpriseTempo", fn)` |

Two shortcuts do what the expert `call(...)` did before: `dog:can_see(actor)` (the dog's own line
of sight) and `player:hurt(amount, dog, "XIII.DTBite")` (the game's own damage, as a bullet's). The
other new functions (`on("fire")`, `is_a`, `trigger`, `ammo`, the screen items) were not needed
by the dogs.

The animations are played at a rate that follows the dog's real speed (speed divided by the
"stride" of each gait, measured on the model), so its paws do not slide whatever its speed.

## Placing a dog

`places.lua` has one block per level. A dog is placed next to a character of the level, its
master, named as the catalog names him (`Mods/_Guide/names/maps.md`):

```text
Dog.place{master = "BaseSoldier5"}
```

Settings of `Dog.place`:

| Setting | What it does |
|---|---|
| `master` | the character the dog belongs to, or `"nearest"`: whoever is nearest to `at` when the dog comes, else the first who comes by; without one the dog guards its place |
| `lying = true` | the dog lies at its place while its master walks his round |
| `at`, `yaw` | where it starts (a point on the ground, `modpos` in the console) and the way it faces; without `at`, next to its master |
| `ground` | what the paws walk on where the level does not say: grass, earth, gravel, snow, tiles, metal |
| `leash = true` | the dog waits: it goes for a player only when it is hurt, when its master fights, or when the level sets it on him with `Dog.sic(dog, player)` |
| `flank = {point, ...}` | points the dog runs through before it goes for the player, the first time |

### A guard the level makes later

Some guards are not in the level when it starts: the level makes them at a given moment (a
`gennmi` actor creates a soldier when its event fires). A dog cannot be tied to someone who
does not exist yet, so the script waits for the same event and places the dog then, next to
whoever has just appeared. On the bridge of Emerald Base (Hual01a), the two guards who walk
from the trucks towards the player come with the event `GuardsArrivalthird`:

```text
on("event", "GuardsArrivalthird", function()
  wait(0.6)                                               -- the level is making them
  local master = Dog.nearest_master({1930, -1570, -390}, 600)   -- where they appear
  if master then Dog.place{master = master} end
end)
```

The events of each level are listed in the catalog (`Mods/_Guide/names/events.md`).

To find a place, stand there in the game and type the console command `modpos`: it prints the
point to give as `at`.

## How far a dog is heard

The game plays a character's sounds at full volume up to its `SaturationDistance`, then fainter
and fainter down to nothing at its `StabilisationDistance`: 400 and 1392 units for everyone
(about 4 and 15 metres). A dog that cries for its master must be heard from further than it
can smell, so `mod.toml` gives the dog's class its own distances (600 and 5500: full up to 6
metres, still heard at 50). The paw steps are played from the place where the dog stands
(`sound(name, dog.position)`), not from the dog, and keep everyone's distances.

## Changing how dogs behave

The settings are at the top of `dog.lua`: how far a dog smells and sees, where a leap starts,
how hard it bites, how fast each gait is, how long it grieves. The tables below them say which
animation and which recording goes with each moment.

## Limits

- A character's collision is an upright cylinder made for a man. A dog is longer than it is
  wide: its muzzle and its hindquarters reach a little beyond the cylinder, into walls and out
  of the bullets' way.
- A dog stays level on a slope: scripts cannot tilt a character.
- The script cannot tell who shot the dog: it goes for the nearest player.
- A dog that runs and gets nowhere steps aside and tries again; it does not find its way
  through a place the level's own paths do not cover.
