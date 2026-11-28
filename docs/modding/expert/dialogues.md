# Dialogues in detail

Every key of `[[map.<Map>.say]]`: game lines, sounds of other levels, your own lines, triggers. Back to [the modding guide](../README.md).

## Dialogues

**Any line and any sound of the game, from any level.** A mod can use every dialogue line and
every sound of the game, on any map: a line of Hual02 on the beach, a helicopter of Sanc03 in
the bank. Nothing of the game is copied into the mod: the mod only names them, and OpenXIII
reads them from the player's own game files, loading the sound bank of the other level when
needed (see [Sounds from any level](sounds.md#sounds-from-any-level)). To find the names:

- the dialogue lines are listed in `dialogues.md`, level by level, with the speaker;
- the sounds are listed in `sounds.md` and the `sounds/` folder, package by package, with the
  banks (levels) that hold each one.

These pages are generated from your copy of the game (see "Reference pages" above).

```toml
# 1. A line of the game, from any level: its voice and its subtitle (in the player's language).
[[map.Plage01.say]]
by = "Pam"
line = "SPADS01_McCall_00"

# 2. A sound of the game from another level (here Sanc03), by its name in sounds.md, with a
#    subtitle written by the mod (optional).
[[map.Plage01.say]]
by = "Pam"
sound = "XIIISound.Vehicles__Sanc3Helico.Sanc3Helico__hSancHelico1Alarm"
text = "Did you hear that?"

# 3. Your own line: your subtitle and a sound file of your mod (see "Your own sounds").
[sounds.MyLine]
file = "sounds/my_line.ogg"

[[map.Plage01.say]]
by = "Pam"
text = "My own line."
sound = "Mod.MyLine"
```

Any character can say a line: one of the game's lines (its voice and its subtitle, in the
player's language), or one of the mod's own (a subtitle, and a voice if you have one). The
character is a `[character]` of the mods placed on the map, or a character of the map itself.

```toml
# A line of the game, said by a character of the map when the level starts.
[[map.Plage01.say]]
by = "Pam"                        # who speaks (see below)
line = "Plage01_Pam_03"           # "Who's that?": a line of the game, by its name (../dialogues.md)
when = "start"
delay = 1                         # seconds to wait first (optional)

# A line of another level, said by a mod character when something happens on the map.
[[map.Plage01.say]]
by = "Carrington"
line = "Hual02_Carrington_06"     # "My best man, alive!"
when = "event"
event = "Objectif91"              # the end of the opening scene (XIII's first goal)

# Your own line, when XIII comes near: a subtitle and, if you want, a voice of your mod.
[sounds.MyLine]
file = "sounds/my_line.ogg"       # your own recording, in your mod folder (see "Your own sounds")

[[map.Plage01.say]]
by = "Carrington"
text = "A beach in Brighton, son? Fine. Find me some cigars, and we will talk."
sound = "Mod.MyLine"              # a sound of your mod, or any sound of the game
distance = 250                    # how near (300 by default)
```

The keys:

- **by**: who speaks. First a `[character]` of the mods on this map, by its name; otherwise a
  character of the map, by its name (`Cine0`), its `Tag` or its `PawnName` (`Pam`). Use the
  console command `modpos` near a character, or the map in the editor, to find these names. A
  line whose speaker is not on the map (or is dead) is not said, and the log says so.
- **line**: a line of the game, `<Level>_<Speaker>_<Number>`. Every line is listed in
  `dialogues.md` (generated with the other reference pages), map by map, with its speaker and
  its English subtitle. Any character can say any line, on any map: the voice is loaded with
  its level's sound bank when needed. `text` may be added to show another subtitle.
- **text** and **sound**: your own line. `text` is the subtitle; `sound` its voice, a sound of
  your mod (`Mod.<Name>`, a WAV or OGG file) or any sound of the game. Either one alone works.
- **when**: `near` (the default): when XIII comes within `distance` of the speaker, out of
  the game's cinematics (XIII does not walk during them); `start`: when the level starts;
  `event`: when the map triggers `event`. An event is the name a trigger, a cinematic or a
  script of the map fires (the `Event` of the actor that fires it, the `Tag` of the actors it
  wakes up): the game's log shows many of them (for example `XIIIGoalTrigger2 trigger`, whose
  Tag is `Objectif91`).
- **delay**: seconds between the moment the line is triggered and the line.
- **once**: `true` by default. `false`: the line is said again each time it is triggered (for
  `near`, each time XIII comes back).
- **from_checkpoint**: as for added actors (see [Maps and checkpoints](maps-and-checkpoints.md#maps-and-checkpoints)).

How the lines are said: as the game's own dialogues. The voice comes from the speaker, the
subtitle is shown by the HUD's dialogue box for the length of the voice plus one second (three
seconds without a voice). Lines never speak over each other, nor over the game's own
dialogues: a line triggered then waits for them to end, and lines triggered together are said
one after the other, in the order of the files. A line whose speaker died before it is said is
not said; the log says so.
The game's own lines do not wait for the mods' lines: one that starts takes over the dialogue
box, as it does for the game's own dialogues.

`[[map.<Map>.say]]` takes map selectors as the other map tables (`[[map."Hual*".say]]`,
`[[map."solo".say]]`); entries add to each other.

The log says, for example, `mods: Plage01: say Hual02_Carrington_06 by Carrington: "My best
man, alive!"` when a line is said.
