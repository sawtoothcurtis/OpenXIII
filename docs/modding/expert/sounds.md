# Sounds in detail

Sounds of any level and their banks, replacing a sound on one map, your own sound files. Back to [the modding guide](../README.md).

## Sounds from any level

Each level loads the game's shared sound bank (`Fix`) and its own bank, named after its level
title (`Sounds/PC/<title>.hxc`). A sound whose resource is in another level's bank plays nothing
there. In a mods build, when a level starts:

- every sound named in a mod value (a class value such as `hFireSound`, the `properties` of an
  added actor or of a `[character]`, a state's `sound`, a `sound` action) gets the smallest bank
  that holds it, if no loaded bank has it;
- every character voice a mod asks for (a `CodeMesh` value) gets the banks that hold that
  voice's sounds;
- every game class a mod brings to the level (an added or replacing actor, a start weapon, a
  given or spawned class, a character's base) keeps its own sounds: a weapon's shot, reload and
  zoom, with those of its attachment and ammo. A sniper rifle added on the beach fires with its
  own sound, without naming any bank;
- the banks of `load_sound_banks` are loaded (see below). They are rarely needed now: only for
  a sound that no mod value names, such as one played by a level script.

The test weapons given by Select level get their sounds the same way.

These banks are loaded once per level and unloaded with the level's own bank. The log says
which bank was loaded for what, with its size and load time, for example
`mods: sound bank Hual04a loaded for sound XIIIsound.Guns__SnipFire.SnipFire__hSnipFire (9632 KB, 11.8 ms)`.
That bank takes about 20 MB more memory at its peak.

```toml
[class."XIII.Beretta"]                                   # the 9mm fires like the sniper rifle
hFireSound = "XIIIsound.Guns__SnipFire.SnipFire__hSnipFire"

[[action]]                                               # a key that plays a sound
key = "J"
sound = "XIIIsound.Guns__SnipFire.SnipFire__hSnipFire"

load_sound_banks = ["SPADS01"]                           # a bank on every map…
[map.Plage01]
load_sound_banks = ["USA01"]                             # …or on one map
```

`sounds.md` lists every sound of the game by package, with the banks that hold it,
and the character voices.

**Character voices.** A character's pain, death and other shouts are chosen by its `CodeMesh`
value (and its timbre, `NumeroTimbre`), in the onomatopoeia sets of the `Onomatopees` package.
A character added on another level keeps a voice if it is given that voice's `CodeMesh`; the
voice's banks are then loaded with the level (../sounds.md lists the voices with the banks they
need). The lines of a level's dialogues are triggered by that level's scripts; a mod can have
any character say any of them on any map (see [Dialogues](dialogues.md#dialogues)), or play one with a
`sound` action. Examples: [make-a-character-talk.md](../make-a-character-talk.md), and
[examples/mods/singing-soldier](../../../examples/mods/singing-soldier/mod.toml), whose soldiers sing a
line of SPADS02a on any level.

Limits: loading a bank takes memory (../sounds.md gives each bank's size) and a short time at
the level start. Creating the bank's resources draws numbers from the game's random sequence,
as the game itself does when it loads a bank: a mod that loads banks can change random events
of the level (never without mods). A `sound` action plays at the player; playing a sound at
another position is not supported yet.

Example: [examples/mods/stronger-pistol](../../../examples/mods/stronger-pistol/mod.toml) (the pistol fires with the sniper rifle's sound, on every map).

### Replacing a sound on one map

Nothing of the game's sounds changes unless a mod says so. To change one on a map, such as an
ambient sound (the background hum or loop an actor of the map plays, its `AmbientSound`):

```toml
# "the game's sound" = "the sound played instead"
[map.Plage01.replace_sounds]
# The helicopter over the beach sounds like the one of Kello01b (a sound of another level).
"XIIIsound.Vehicles__PlgHelic.PlgHelic__hInit" = "XIIISound.Vehicles.KelloHelicoLoop1"

[map.Banque01.replace_sounds]
# The bank's generator hums with your own recording (see "Your own sounds").
"XIIIsound.Ambient.Generator1" = "Mod.MyHum"

[sounds.MyHum]
file = "sounds/my_hum.ogg"
```

- On that map, every time the game plays the first sound, the second one plays instead: the
  ambient sounds of the map's actors, and every sound played by its name (scripts, weapons,
  class values). Other maps keep the game's sound.
- The bank of the new sound is loaded with the level if needed, as for the other sounds of
  mods.
- To find the names: `maps.md` lists the ambient sounds of each map (the `AmbientSound` values
  of its actors); every sound of the game is listed in `sounds.md`.
- `[map."*".replace_sounds]` replaces it on every map; `[map."Hual*".replace_sounds]` on the
  maps whose name starts with Hual.
- To replace a sound everywhere by your own file, `replace` in `[sounds.<Name>]` also works
  (see "Your own sounds").

## Your own sounds

A mod can bring its own sounds, as WAV (PCM 8, 16, 24 or 32-bit, or float) or OGG Vorbis files
of the mod folder, mono or stereo, at any sample rate:

```toml
[sounds.BlasterShot]           # a new sound, "Mod.BlasterShot" (examples/mods/blaster-gun)
file = "sounds/blaster-shot.wav"

[class."Mod.Blaster"]
hFireSound = "Mod.BlasterShot" # the Blaster fires with it

[sounds.Shot]                  # a new sound "Mod.Shot", which also plays instead of a game sound
file = "sounds/shot.wav"
replace = "XIIIsound.Guns__9mmFire.9mmFire__h9mmFire"

[[action]]
key = "J"
sound = "Mod.BlasterShot"      # a key that plays it
```

- `Mod.<Name>` works wherever a sound name is accepted: class values (`hFireSound = "Mod.BlasterShot"`),
  `properties` of added actors and characters, a state's `sound`, `sound` actions.
- `replace` plays the mod's sound each time the game plays that sound, on every level (its
  bank is loaded if the level does not have it). A game sound can choose among several
  recordings (a shot has variants): each of them plays the mod's sound. The game keeps its own
  timing of the replaced sound (its original length). To replace a sound on one map only, or
  by another sound of the game, see [Replacing a sound on one map](#replacing-a-sound-on-one-map).
- The files are read once when the game starts; a missing or unreadable file gives one warning
  line and that sound is left out.

Example: the Blaster's firing sound and its "empty" sound (`hNoAmmoSound`) in
[examples/mods/blaster-gun](../../../examples/mods/blaster-gun/mod.toml).

## Co-op characters' voices

In co-op, a player who chose another character than XIII shouts with that character's voice:
the game's hurt and death shouts of the character (the same ones the character has as an
enemy or ally; Kim and Amos get the shouts the game has for them but never plays). The noises
stay XIII's (impacts, heartbeat, the medikit, the jump's effort). The voice sounds only XIII
has in the game (jump, landing, end of a run, ladder, panting) are silent for the others.

A mod can fill them, or replace the game's shouts, with sounds named
`Coop_<Character>_<Kind>`, a number after it for several recordings (one is drawn each time):

```toml
[sounds.Coop_Jones_Jump1]
file = "sounds/jones-jump-1.wav"

[sounds.Coop_Jones_Jump2]
file = "sounds/jones-jump-2.wav"

[sounds.Coop_Kim_Hurt]
file = "sounds/kim-hurt.wav"
```

- `<Character>`: the character's id (XIII, Jones, Carrington, Mongoose, Kim, Amos, McCall,
  Sheridan, Standwell, Admiral, Spads, GI, FBI, Killer, Conspirator, Policeman), or one outfit's
  code (`Killer.3`: the fourth killer), which wins over the id.
- `<Kind>`: Hurt, Death, Jump, Land, RunEnd, Ladder, Breath.
- XIII's own voice is never changed, nor the solo game.

