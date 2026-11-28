# Characters in detail

Class values of characters, changing one placed character, `[character.<Name>]` tables, AI states and keys. Back to [the modding guide](../README.md).

## What the simple keys become

The simple keys of [spawn-a-character.md](../spawn-a-character.md) and
[change-a-character.md](../change-a-character.md) are translated, when the mods are read, into
the raw values below; nothing else runs for them. Raw `properties` written in the same entry
come after them and win.

| Simple key | Raw value |
|---|---|
| `character = "<look name>"` (no `[character]` of that name) | a character with `base = "XIDCine.Cine2"`, `team = "friend"`, the look's `Mesh`, its voice (`CodeMesh`) and its name (`PawnName`) |
| `look` | `Mesh` (the friendly names are those of the catalog's Characters tab) |
| `skin` | `Skins = [<skin>]` (`Skins` is a list: the first skin) |
| `health`, `name` | `Health`, `PawnName` |
| `voice` | `CodeMesh`: a number, a name of the game's skin list (Carrington, Densetsu…), or a look name (the voice most characters of that look have in the maps) |
| `weapon` | `InitialInventory[0] = (Inventory = Class'<weapon>', Count = 2)`, given by `BaseSoldier.InitializeInventory` when the character's AI starts |
| `behaviour` | `base`: `stands` = `XIDCine.Cine2`, `patrols` and `fights` = `XIDPawn.BaseSoldier`; `fights` also sets `team = "enemy"`, `stands` `team = "friend"` |
| `team`, `can_die` | the `[character]` keys of the same names |

On a character the map places (`change`), `team` and `can_die` make it a mod character once
its AI controller exists (end of the level's start-up); `behaviour` is refused there, since the
character keeps its class and its role.

## Changing characters

Characters (enemies, allies, the player) are classes too, so the same tables change their
stats: health, speed, sight, aggressiveness, aiming time… `characters.md` lists
the character classes and their properties with the original values.

```toml
[class."XIDPawn.BaseSoldier"]   # the soldiers
Health = 200                    # original: 150
TempsVisee = 0.5                # aiming time before shooting
apply_to_placed = true          # also the soldiers the maps placed with their own values

[map.Plage01.class."XIDPawn.BaseSoldier"]
Health = 300                    # only on Plage01
```

Three things to know, because of how the levels are built:

- **A character's look comes from the map.** `XIDPawn.BaseSoldier` (and several other classes)
  has no look of its own: its default mesh is XIII's (`XIIIPersos.XIIIM`), and each soldier
  placed in a map gets its `Mesh` from the level designer. A character a mod adds must be given
  one, for example `properties = { Mesh = "XIIIPersos.SpadsM" }` (the soldiers of the beach);
  otherwise the log warns that it is drawn with XIII's own look. The class tells what a
  character does, not what it looks like: `characters.md` gives the default look of each class
  (XIII's is marked), and `character-models.md` says who wears each model in the maps (the
  model names are short or French words: `GardienM` is a guard, `SpadsM` a beach soldier…).
- **Most enemies are one class**, `XIDPawn.BaseSoldier`, set up one by one in the maps. A
  subclass inherits a changed value unless it sets its own (`XIDPawn.Johansson` keeps its
  health of 2000).
- **Characters placed in a map often carry their own values**, saved with the map by the level
  designer (a soldier with 125 health, a scripted character with 10000 so it cannot die).
  A class value alone does not reach them. Add `apply_to_placed = true` to the class table to
  write the value on every actor of the class (and its subclasses) that the map placed, when the
  level starts. This also changes the scripted characters: a 10000 that kept one alive for a
  cutscene becomes your value.
- **Enemies created during play by a generator** (`XIDPawn.gennmi`) get their health
  (`PV_NMI`), sight, skill, aggressiveness and aiming time from the generator placed in the map,
  not from their class. Change the generator, with `apply_to_placed = true` since generators
  are placed actors:

  ```toml
  [class."XIDPawn.gennmi"]
  PV_NMI = 200
  apply_to_placed = true
  ```

Map tables, `[map.<Map>.class."Package.Class"]`, apply only on that map (the map file name
without `.unr`) to the actors it placed, of the class and its subclasses. They win over the
global values for that map. They do not change the class defaults, so characters created later
during play (by a generator, for example) do not get them: change the generator in the map table
instead.

When a level starts, the log sums up each value written on placed actors, for example
`mods: Plage01: XIDPawn.BaseSoldier.Health = 300 on 7 actor(s) placed in Plage01, 4 of them with
their own value in the map (…)`. Values are written before any script of the level runs. A saved
game loaded later already contains them and is not changed again (a wounded soldier stays
wounded).

A complete example is in
[examples/mods/tougher-soldiers](../../../examples/mods/tougher-soldiers/mod.toml).

### Changing one character of a map

To give one character of a level another look (or other values) while it keeps its place and
its role in the story:

1. Start the level with mods, walk up to the character, look at it, open the console and type
   `modwho`. It prints (on screen and in `mods-log.txt`) the actor's name, class, `PawnName`,
   `Tag`, `Mesh` and `Health`, and an entry ready to paste.
2. Paste the entry in your `mod.toml` and choose the look:

```toml
[[map.PRock03.change]]
who = "gardien_lostpass"    # the guard who loses his pass
look = "Jones"
```

That is all: in PRock03, the guard looks like Major Jones and does everything he did before.

The keys:

- `who`: the actor's name as `modwho` prints it (`Cine2`), else a character's `Tag` or in-game
  name (`PawnName`), else another actor's `Tag`; case does not matter. Characters come first
  because the other actors of their scene often share their `Tag`. If several actors match, all of them change
  and the log says how many: add `near = [x, y, z]` (and `radius`, 100 by default) to pick
  the ones around a point.
- `look`: a character model, in full (`"XIIIPersos.JonesM"`) or by a short name: the model's
  name without `XIIIPersos.` and the final `M` (`"Spads"`, `"Gardien"`), or a friendly name
  such as `"Jones"`, `"Carrington"`, `"Mongoose"`, `"Kim"`, `"XIII"`, `"Guard"`, `"Soldier"`.
  [character-models.md](../character-models.md) says who wears each model. The skins the map gave
  the character are taken away (they were made for the old model); `skin = "Package.Group.Name"`
  draws another one on the new model.
- `health = 300`, and `properties = { ... }` for any other value of the actor, as for added
  actors.
- `from_checkpoint = N` and map selectors (`[[map."Hual*".change]]`) as for the other map
  entries.

A name or a look that does not exist changes nothing; the log says so with the closest valid
names (`did you mean gardien_lostpass?`). Each change made writes one line in the log, for example
`mods: PRock03: change gardien_lostpass: look Jones: Cine2 (XIDCine.Cine2)`. Changes are written
before any script of the level runs, again at each load of the level, and not on a saved game
(it already contains them). In a network game the host makes them, and the guests get the
characters from the host. The game itself does not replicate `Skins`: the mods build sends the
skins a mod chose to each guest (see [content-and-network.md](content-and-network.md#mods-in-network-games)).

The same example, step by step: [your first mod](../README.md#your-first-mod-in-5-minutes).


## Keys and character behaviour

A mod can bind keys to actions, and define its own characters from any character class of the
game, with a team, the states their AI may be in, and an animation or a sound per state.

```toml
[[action]]
key = "K"                      # F1…F12, A…Z, 0…9, NumPad0…NumPad9, Insert, Delete, Home, End,
                               # PageUp, PageDown
spawn = "Guard"                # a [character] of the mods, or any class "Package.Class"
in_front_of_player = 250       # distance (150 by default)
# or: give = "XIII.Bazook"     # a weapon for the player
# or: command = "modpos"       # a console command, as if typed by the player

[character.Guard]
base = "XIDPawn.BaseSoldier"   # any character class (../characters.md); give it a look:
                               # BaseSoldier alone looks like XIII himself
team = "neutral"               # friend | neutral | enemy: how it treats the player
allowed_states = ["ResteSurPlace"]
can_die = true                 # false: cannot be hurt or killed
death_animation = "DeathDos"   # optional
properties = { Health = 200 }  # optional, as in [class] tables

[character.Guard.state.ResteSurPlace]
animation = "WaitNeutre"       # an animation of the character's mesh
sound = "Package.Group.Sound"  # optional: a sound of any level
loop = true                    # true by default: the animation and the sound start again
                               # when they end
```

How it works:

- **Keys.** Only in a mods build. A key that the game window already uses (F3, F4, F8, F9, and
  F5, F6, F7, F11 in the cheat build: the log warns when the game starts) or that the player's own settings bind to a game command is left to
  them: the mod action is not run, and the log says so once. The console command
  `modkey K` runs the action of a key as if it were pressed.
- **Spawn.** The actor is created on the ground, `in_front_of_player` units ahead in the
  direction the player looks, facing the player; where there is no room, at the first spot
  around that point with room (150, 250 then 350 units away). A character gets its AI controller exactly
  like the characters placed in the maps, and its `properties` count from the first moment.
- **States.** `allowed_states` applies to the character's AI controller: the AI first runs its
  start-up state (it sets itself up there), then every time it asks for a state that is not in
  the list, it goes to the first state of the list instead. A dead character follows the game's
  death states. The state names of each AI are listed in `characters.md`.
- **Looks.** The animation (and sound) of a state starts when the AI enters that state; a
  looping animation is kept on while the AI stays in it, even if the state's own code plays
  another one, and stops when it leaves. It moves the whole body: the waiting pose the game
  blends over a soldier's upper body stays off while it plays. When the AI asks for another
  state and is kept in the one it is in, its look goes on as it is (the sound is not started a
  second time). A wrong animation name prints the list of the mesh's
  animations in the log. `death_animation` plays when the character starts dying.
- **Sounds.** The sound of a state plays from the character; a sound of any level (its sound
  bank is loaded when needed). With `loop`, a recording (a line, a song) starts again when it
  ends; a sound program of the game (most `XIIISound` sounds) plays once. The sound stops when
  the character leaves the state, dies or is taken away. The `restart` action (reactions.md)
  starts the animation and the sound again from the beginning.
- **Team.** XIII's soldiers always treat the player as an enemy; `team` changes that for the mod
  character only (friend 1, neutral 0, enemy -1 in the game's alliance levels). Other
  characters treat it as they treat any soldier.
- **can_die = false** uses the controller's own "god mode" (`bGodMode`): no damage, no death.

Limits of this version: characters cannot be given routes of their own; actions and mod
characters have only been tried in the solo game.

A complete example, with its keys written in a script, is in
[examples/mods/singing-soldier](../../../examples/mods/singing-soldier/mod.toml).

### Adding a character to a map

A `[character]` can also stand on a map from the start of the level, placed like any added
actor but with `character` instead of `class`:

```toml
[character.Carrington]
base = "XIDCine.Cine2"                 # the game's class for story characters
team = "friend"                        # never attacks the player
properties = { Mesh = "XIIIPersos.CarringtonM", CodeMesh = 16, PawnName = "Carrington" }

[[map.Plage01.add]]
character = "Carrington"
position = [-470, -860, 1190]          # a point on the ground (console command modpos)
yaw = 180                              # 0 east, 90 south, 180 west, 270 north
# from_checkpoint = 2                  # optional, as for any added actor
```

What each part gives:

- **Class**: `XIDCine.Cine2` is the class of the game's story characters (Carrington, Janet,
  McCall… in their levels): they stand still and act only when the level's scripts tell them
  to, so a mod character of this class stays calm. A soldier class (`XIDPawn.BaseSoldier`) has
  the soldiers' AI instead: it patrols, hears and fights; keep it calm with `team` and
  `allowed_states` (see above).
- **Look**: `Mesh`, a character model (`character-models.md` says who wears each one). Give one, or the
  character looks like XIII (see "Changing characters").
- **Voice**: `CodeMesh`, the voice of its shouts (pain, death, `sounds.md`, "Character
  voices"); its lines of dialogue are `[[map.<Map>.say]]` entries (see [Dialogues](dialogues.md#dialogues)).
- **Name**: `PawnName`, the name the game's dialogues know a character by. Optional.
- **Animation**: per state, an animation of its mesh (`animations.md`).
- **Where**: `[[map.<Map>.add]]` with `character`, or a key action with `spawn`.

The log says `mods: Plage01: add Character.Carrington at …` when the level starts.

[make-a-character-talk.md](../make-a-character-talk.md) shows a character of another level
(Carrington, from Hual02) brought to Plage01, where he says his real lines of the game, with his
own voice and subtitles read from the player's game files.
