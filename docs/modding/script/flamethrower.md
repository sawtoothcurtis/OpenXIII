# A weapon with a script: the flamethrower

`examples/mods/flamethrower` adds a flamethrower. The weapon itself is a `mod.toml` class
(like the Blaster); what makes it a flamethrower is a script: the jet of flames, the characters
it sets on fire, and the black marks it leaves. Everything it shows is made of the game's own
effects.

## What the game already holds

| What | Where | |
|---|---|---|
| A human torch | `XIDCine.XIIIFireTorchEmitter` | an emitter of flames and of the "aahh" word, lit by a trigger and put out by a second one |
| The burning man's flames | `XIIICine.effets.flammeperso` | the texture of the man on fire of the rooftops (Toits01) |
| His panic | `Affole` | the soldiers' animation the burning man plays while he runs |
| Fire damage | `XIII.DTFire` | a damage type: no armour stops it, no blood |
| A blast mark | `XIII.GrenadBlast` | the black mark a grenade leaves on the ground |

On the rooftops (Toits01) a man runs on fire: a story character (`Cine1`) wearing three
emitters by their `AttachTag` (flames, smoke and the "aahh" word) and playing `anim loop affole`.
The mod gives the same look to whoever its flame touches.

## The files

```text
flamethrower/
  mod.toml                  the weapon (made from the mini Uzi), its back tanks and the effects
  models/                   the steel lance and the tanks (../make-flamethrower-model-v2.py)
  textures/                 charred wood (../make-flamethrower-textures.py) and the HUD icon (a
                            picture kept as it is, rendered once from the first model)
  scripts/flamethrower.lua  the flame, the burning, the fires, the marks, the blast
```

The game ships it as an example, in `Mods/_Examples/flamethrower`: like the other examples it is
not active. Copy the folder into `Mods` to turn it on at the next launch; it is then a mod like any
other, listed on the Mods page (where it can be turned off) and free to be changed.

Once on, every player has the flamethrower in hand at the start of each level, its fuel filled up
to `start_fuel` (`give_at_start = false`: give it another way, with `player:give("Mod.Flamethrower")`,
a `[[map.<Map>.add]]` pickup of `Mod.FlamethrowerPick` or a start weapon of a level). Hold the fire
button: the flame burns while it is held and uses fuel (100 at a time, 600 at most). With the
flamethrower in hand, L fills the fuel up (`refill_key`; false: no key).

The weapon is the mini Uzi (the fire button held down fires again and again, and uses fuel),
seen in first person as the 9mm's hands holding the tube. In third person it is held level in
both hands as the M16 (`FiringMode = "FM_M16"`, `ThirdPersonRelativeRotation` lifting the lance
to the M16's angle in the hand): the mini Uzi's own way, hanging at arm's length, would point the
lance at the ground. Its "bullets" go no further than the
flame (`TraceDist = 650`) and hurt nobody (`SRDamages = 0`): the script's flame burns.

The effects of `mod.toml` are all made from the human torch, each with its own values:
`Mod.FlameJet` (fast flames shot where it faces, slowing down, rising and growing, stopping at
walls), `Mod.PilotFlame` (a small flame at the nozzle while the weapon is in hand),
`Mod.BurningBody` (flames over a whole character, and the "aahh" word), `Mod.SceneryFire`
(a fire left on a wall or a floor, with smoke) and two marks made from the grenade's,
`Mod.FlameScorch` and `Mod.FireScorch` (wider), which stay as long as the level.

## What the script does

Every frame, for each player holding the flamethrower:

- the jet and the pilot flame are put at the nozzle (in front of the eyes, a little to the
  right and below) and turned where the player looks: `fx:call("SetLocation", ...)`,
  `fx:call("SetRotation", ...)`; the jet's flames are switched on while the fire button is held
  (`fx:set("Emitters[0].Disabled", false)`);
- the characters within the flame's cone, with nothing in between (`trace`), catch fire: a
  `Mod.BurningBody` carried by the character (`effect{class = "Mod.BurningBody", on = c}`), lit by
  its own trigger. A character on fire burns until he dies, and his body burns 25 seconds more;
  both turn black little by little (`c.charred`, from 0 to 1 in 7 seconds), with what he wears (a soldier's vest and helmet, a beret: the game's attachments that are not weapons), never the weapons. Seven soldiers out
  of ten panic: they run from whoever burns them (their AI's own flight, the controller's state
  `Fuite`), given again whenever they stop;
- where the flame meets the scenery (`trace`: the level, its static meshes such as a pier, its
  movers and objects), a black mark is left (`Mod.FlameScorch`). The game's mark lets go of its
  picture after one second (`AbandonProjector`): the script attaches it again
  (`mark:call("AttachProjector")`) and keeps it as long as the level (`LifeSpan` 0). Now and then a
  `Mod.SceneryFire` (flames and the smoke of the burning man's torch) burns there for 8 to 14
  seconds, carried by the object it is on, and leaves a wider mark (`Mod.FireScorch`). Objects
  hit get darker (`charred`) and take fire damage (a crate breaks, a barrel blows up);
- the game's own fire sounds are played from the flames of what burns, never from the
  character or the jet: a roaring fire (`XIIISound.Ambient.Sanc02bFire2`) once when the fire
  starts, and crackles (`SancFire`, `FireSpad1`) from a silent emitter carried with the flames; some of them loop, and the game
  stops the looping sounds of an actor only when it is taken out of the game, so they stop with
  the flames (`StopActorSounds` before each new play and when a fire dies out). The jet hisses
  with the steam of the submarine levels (`XIIISound.Ambient.SteamLoop3`), looping from a silent
  emitter at the nozzle that is taken out of the game as soon as the flame stops; each press of
  the fire button also plays the M16 grenade launcher's shot (softer) and the thud of XIII hit by
  a grenade. The game has no volume per sound: a softer sound is played from an emitter whose
  distance falloff (SaturationDistance, StabilisationDistance) makes it sound further away.

Fires left on the scenery hurt whoever stands in them: characters catch fire, players lose
about 12 health per second (the jet itself never hurts a player, its own holder included). A
body carried on a player's shoulder is drawn by another actor (the left hand's `TheHeldBody`):
the script gives it the `charred` value of the body it shows. Over the jet, over burning
characters and over the fires, the view waves a little (`heat{}`, a mods build addition drawn
by the game's own screen-effects path).

What burns gives off the black smoke of the Sanctuary's fires (`Mod.BlackSmoke`). The flame
heats the area it touches (cells of 400 units); an area that has taken six seconds of flame
bursts into a blaze at points found around it with `trace`: its walls inside and out, its
ceiling, its roof and the deck around (lines down from above). Wood (and cloth) burns fast: it is
recognised from `trace`'s `material` (`XIIIPlage.PLwood05`, `PLroof01`...) or from the name of
the object hit or of its mesh (a list of words at the top of the script). Wood heats two and a
half times faster, burns longer, and the fire creeps along it from where it started (faster
upwards than sideways, slowly downwards), never across what is not wood. At most three blazes
burn at once; each heats the areas next to it, which may catch in turn.

A blaze grows in stages, on the game's clock:

1. small flames where the jet hit (`Mod.SceneryFire`, the burning man's flames);
2. the fire creeping over the structure: at each point it reaches, tall upright flames
   (`Mod.Blaze`: the Sanctuary's animated `XIIICine.feuAD00`, rising and dying in two seconds,
   tinted yellow-orange to red), and five seconds later the grey smoke of the Sanctuary's fire
   that darkens what is behind it (`explogrenadeB`, `PTDS_Darken`, growing from nothing); a few
   fine sparks already;
3. once most of its points burn and it has burned twelve seconds, its full fury: fine sparks
   raining from what burns above (`Mod.SparkRain`) and rising high above the roof, drifting
   with the air (`Mod.SparkRise`), as many as `particle_amount` says; the Sanctuary's sparks and
   falling flamelets (`Mod.Sparks`, `Mod.Flamelets`); and, set back behind the flames, a few of
   the Sanctuary's great clouds of fire (`Mod.FireCloud`: `extinct_exploAD`, orange turning red
   over twelve seconds, rising slowly, not turning). Its emitters appear over a couple of
   seconds and fade in with their own fades;
4. twelve seconds before it is out it calms down (the fury stops coming and fades), then its
   flames die out.

Everything a blaze burns turns black over thirty seconds and stays black as long as the level:
the scenery's meshes, doors and objects around its points (`actors_near`, `charred`: a mesh is
drawn with its own textures, darker), the bodies lying there, and the level's own floors, walls
and ceilings (`char_area`: the surfaces of the map within 260 units of each flame are darkened);
it leaves wide burn marks (`Mod.CharredSurface`) and its scorch marks, which never go. A piece of
scenery bigger than `whole_char_size` (9 m: Winslow Bank's hall is a single mesh, so are
Plage01's pier and its luncheonette) only turns black as a whole once the fire has really taken
it: a building blew out where its flames touched it (within `blast_building`), or its flames have
been in enough areas of 4 m around it (`whole_char_cells` for every 4 m of its size, from the
actor's `size`). A burst of the jet or a small fire in a huge hall leaves only its marks; a hut
or a luncheonette that burns down turns black all over. The jet's
own small marks stay too, 400 at most (the oldest of the jet's go first, never a blaze's).
Every character alive takes the fire's damage through the game's own `TakeDamage`, as a bullet
would: the game decides whether it can be hurt. Only the soldiers of the game's AI
(`IAController`) panic; a character of a scene (`CineController2`) keeps doing what the level
makes it do. Bodies already lying (Pam in the hut of Plage01) are only blackened.

A blaze is heard in layers, each a loop played from its middle with its own reach (the game's
attenuation of an actor's sounds: `SaturationDistance`, `StabilisationDistance`,
`StabilisationVolume`, set from the script), so that near it sounds full and far away only its
distant roar remains (`blaze_layers` at the top of the script): close by, the roar of the
Sanctuary's fire (`Sanc02bFire2`) while it takes hold and calms down, its fiercer loop
(`SancFireLoop`) in its fury; a middle layer (`Sanc02bFireLoop`) all along; and in its fury and
after, the burning helicopter's loop (`HelicoFireLoop`), heard to about 95 m. The Sanctuary's
fire start (`Sanc02bFireStart`) is played once when the fury begins. Crackles follow the stages
(`FireSpad2`, `FireChimney1` with a softer `FireSpad0` once the fury has lasted, `FireSpad2`
again), and `FireSpad1` while the embers glow. Every layer is played again every few seconds (the
game drops a looping sound too far from the player and does not start it again); a new layer
starts before the one it replaces stops. Small fires and burning characters keep a short roar
and crackle.

The heat breaks the objects around a fire that a bullet breaks, through the same `TakeDamage`
with the `XIII.DTFire` damage type, less and less with distance (`heat_props` at the top of the
script): bottles and glasses (`XIIIDecoPickup`, broken at the first hit), windows, shutters,
panels and extinguishers (`XIDCine.BreakableMover` and `Fenetre`, broken when their health is
gone, unless the level made them unbreakable: `BreakType` 1). Each breaks with its own effect and
sound. Objects whose breaking sets off an event of the level (their `Event` is set) and what
someone holds are left alone.

The flamethrower is a steel lance held in both hands, and two fuel tanks with a pressure bottle
worn on the back while it is in hand (`Mod.FlamethrowerTanks`, made from the game's vest,
`XIII.ArmorAttachment`, put on the spine bone with `AttachToBone`; its owner does not see it).
Both are made with Blender by `../make-flamethrower-model-v2.py`.

None of this is saved: a saved game, a checkpoint or a new level brings the scenery back as the
game draws it.

A character on fire loses health with the game's own `TakeDamage` and the `XIII.DTFire` damage
type: 40 per second in the flame, 12 per second after it. Damage is counted from the game's
clock (`time()`): the same at any frame rate. The game multiplies a hit by its own rules (a
soldier on alert loses three times the damage given): the script measures the health really
lost and gives less, so that the rates are the ones written. The settings are at the top of
`flamethrower.lua`.

## Two jets: first person and seen by the others

The jet the player sees starts at the nozzle in front of his eyes (`nozzle` at the top of the
script) and only he sees it (`bOnlyOwnerSee`). With more than one player (split screen or
network), a second jet starts from the tip of the lance held in his hands, as the others see
him, and he does not see it (`bOwnerNoSee`). It is pinned there (`pin_to`) on the gun's
third-person actor (`Weapon.ThirdPersonActor`), at `muzzle_tip` (forward, right, up in the
lance's own frame): each machine starts it at the tip of the lance as it draws it, also on a
network guest (the place goes once, then each guest puts it on its own drawing of the lance
every frame). It is turned from the tip towards the point the player aims at, `aim_point` units
in front of his eyes: what he aims at burns, and the jet leaves the lance there. The game holds
every gun level in a character's hands whatever his view's pitch (the M16 too: only his head
and chest follow it), so a player aiming high or low is seen with a level lance and a jet
rising or falling from its tip. Without the gun's actor (a moment while the weapon is changed),
the jet starts in front of his eyes (`muzzle_third`). With
an object in hand (a medikit, the grapple, a key: `bWeaponMode` false) there is no jet.

## No light from the effects

The human torch the effects are made from lights what is around it. In the game, an actor's light
falls only on characters and on the scenery that moves (doors, shutters, a medikit box), never
on the level's walls: a door lit by it looks pale next to its wall. So the mod's effects give no
light (`LightType = "LT_None"` in `mod.toml`), except the flames on a burning character, like the
game's burning man.

## Smoke

A blaze in its fury gives off thick black smoke (`Mod.FurySmoke`, `fury_smoke`); smoke hangs
under the ceilings of a building that burns (`Mod.CeilingSmoke`, `ceiling_smoke`), and once the
fire is out, smoke keeps rising from what burned for `after_smoke_seconds` (`Mod.AfterSmoke`).

## The building that blows out

A building (the scenery found around the first blaze on wood, within `blast_building`) is
watched while it burns: points on its walls, floor and roof (found with `trace` from the first
blaze) count as burnt once a flame has come within `blast_reach_sample` of them. When
`blast_share` of them burn (three quarters) and the fire has been in its fury for at least
`blast_min_seconds`, the building blows out, once:

- the Sanctuary's explosion (`ExSanc1`) then, a moment later and softer, the roof explosion of
  the rooftops (`ExToit1`), as `blast_sounds` says (sound, delay, decibels lower), heard from
  far away (`blast_sound_range`);
- the game's grenade explosion and a wave of heat shimmer; characters and players
  within `blast_reach` hurt (`blast_damage` at its middle) and pushed;
- every window breaks with flames bursting out of it (`Mod.WindowBurst`), and with
  `blast_breaks_all` also those the level uses as a signal; bottles, glasses and the radio
  break;
- the whole building catches: every part of it turns black, and new blazes start inside and
  along its outer walls.

## How long it burns

A hut takes about a minute from the first flame to the blast: an area takes `blaze_heat`
seconds of flame to catch, a blaze burns `blaze_seconds`, at most `blazes_max` at once, each
giving `blaze_spread` heat a second to the areas next to it; its fury comes after
`inferno_after` seconds once `inferno_share` of its flames burn; it calms down `calm_before`
seconds before it is out, and small flames (`last_flames`) die out one after the other where it
burned. The fire may then creep on along the wood that touches the building (a pier) for a
minute or two more.

## Settings

Every setting is at the top of `flamethrower.lua` (the `Flamethrower` table), each with its
comment. The main ones:

| Setting | What it changes |
|---|---|
| `range`, `cone`, `width` | how far and how wide the flame reaches |
| `flame_damage`, `burn_damage`, `char_seconds`, `corpse_seconds`, `panic_chance` | the burning of characters |
| `prop_damage`, `heat_props` | damage to objects in the flame, and the heat that breaks objects near a fire |
| `fire_every`, `fire_seconds`, `fires_max`, `fire_tongues` | the small fires left on the scenery |
| `blaze_heat`, `blaze_seconds`, `blazes_max`, `blaze_spread`, `inferno_after`, `inferno_share`, `calm_before`, `last_flames` | how blazes start, grow, spread and die |
| `wood_words`, `wood_heat`, `wood_blaze_seconds`, `wood_creep` | what counts as wood, and how much faster it burns |
| `fire_clouds`, `sparks`, `particle_amount`, `body_sparks`, `tall_flames` | how much a blaze shows (fewer particles for a slow computer) |
| `fury_smoke`, `ceiling_smoke`, `after_smoke_seconds` | the smoke |
| `char_scenery`, `char_scenery_seconds`, `char_area_radius`, `char_mark_spacing`, `char_marks_max`, `marks_max`, `whole_char_size`, `whole_char_cells` | the black left by the fire, and the marks |
| `blaze_layers`, `blaze_kick`, `blaze_crackles`, `roar_sounds`, `jet_sounds`, `jet_loudness`, `burst_sounds` | the sounds |
| `blast`, `blast_share`, `blast_min_seconds`, `blast_reach`, `blast_damage`, `blast_breaks_all`, `blast_sounds` | the blast of a burning building (`blast = false`: none) |
| `ground_fire_reach`, `ground_fire_damage` | what a fire does to a player standing in it |
| `heat_strength` | the heat shimmer (0: none) |
| `nozzle`, `muzzle_tip`, `muzzle_third`, `aim_point`, `tanks` | where the jets start, where the others' jet goes, and where the tanks are worn |
| `weapon`, `give_at_start`, `start_fuel`, `refill_key` | the weapon's class, given at each start with its fuel, and the key that fills it up |
| `burst_gap`, `sound_every`, `burn_sounds`, `fire_sounds` | how often the shots' and fires' sounds play, and which |
| `scorch_every`, `scorch_spacing`, `mark_seconds` | the jet's marks: how often, how far apart, how long (none: as long as the level) |
| `fire_spacing`, `fire_spread`, `fire_spread_seconds` | the small fires: how far apart, and their tongues around them |
| `blaze_cell`, `blaze_reach`, `blaze_points`, `blaze_damage_reach`, `smoke_after` | a blaze's size: its areas of heat, how far it finds walls and roof, its flames, where it hurts, when its smoke comes |
| `wood_spread`, `charred_wood`, `charred_seconds`, `charred_max`, `embers`, `embers_seconds`, `wall_glow`, `smoke_column` | more effects, some off by default: charred planks, embers, the flames' glow on walls, a column of smoke |

## In a network co-op game

Every player needs the mod (the same `Mods` folder, as for any mod: the host refuses the others
with a message). The script runs on the host; the guests see what it makes (the jets, fires,
smoke, sparks, marks, the black of what burned and the blast, with their sounds) through the
mods build's sharing of script effects (see "In a network game" in [the functions](functions.md)).
A guest's own jet is placed at his own view on his side, without delay; the jets of the others
follow them. A guest who joins after a fire finds the scenery as black as the host sees it.

The script gives the jet to a guest while the host sees the guest's gun fire, shot after shot
(`on("fire")` hears each of those shots on the host), until the guest lets go of the button: the
jet may last up to a second after the button is let go.

```lua
-- The shots of each player's gun (on the host, a network guest's too).
local shot_at = {}
on("fire", function(p) shot_at[p.name] = time() end)
```

The fuel is the gun's ammunition: `p.ammo` reads it and `p.ammo = 600` fills it up (the `L`
key). The fire hurts with `c:hurt(n, by, "XIII.DTFire")`, which gives the health really lost (the
game's rules change a hit: difficulty, armour). The effects are lit and put out with `fx:trigger(by)`, and what a flame meets is sorted with
`a:is_a("Pawn")`, `a:is_a("Mover")`... A weapon given by a script to a guest is not put in his hands by the game: he takes it with
his weapon keys.

## Not done yet

- The lance is held in the 9mm's hands.
- The motionless figures some levels place as decoration (`XIDPawn.gennmi`, the guards of
  Hual02 that cannot be picked up) are not darkened: they are not characters for the game and
  are drawn another way.
- Nothing burnt is saved: a saved game or a checkpoint brings the scenery back as the game draws
  it.
- A network guest is given the weapon but takes it in hand with his weapon keys, and his jet may
  last up to a second after he lets go of the button.
- The burning man of Toits01 is not darkened by the game's data (his skin is an ordinary one):
  `charred` is a mods build addition, drawn by darkening the actor's cartoon colour.
