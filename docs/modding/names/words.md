# Fixed words

Generated from OpenXIII's own tables: every name below is accepted as written.

## team

`team = "…"`: how a character treats XIII.

| Word | Meaning |
|---|---|
| friend | never attacks XIII |
| neutral | leaves XIII alone |
| enemy | attacks XIII |

## behaviour

`behaviour = "…"`: what a character a mod adds does.

| Word | Meaning |
|---|---|
| stands | a calm story character that stays where it is (the default) |
| patrols | a soldier with the game's AI |
| fights | a soldier that attacks XIII |

## when (dialogues)

`when = "…"` of `[[map.<Map>.say]]`: when the line is said.

| Word | Meaning |
|---|---|
| near | when XIII comes within `distance` (the default) |
| start | when the level starts |
| event | when the map triggers `event = "…"` (see [events.md](events.md)) |

## Map selectors

The `<Map>` part of `[map.<Map>…]`.

| Word | Meaning |
|---|---|
| Plage01 | one map (see [maps.md](maps.md)) |
| Hual* | every map whose name starts with Hual |
| solo | every map of the story |
| multi | every multiplayer map |
| * | every map |

## Keyboard shortcuts

One of these settings in each `[[shortcut]]` block (see [keyboard-shortcuts.md](../keyboard-shortcuts.md)).

| Word | Meaning |
|---|---|
| spawn | a character appears in front of the player |
| give | a weapon for the player |
| sound | a sound plays at the player |
| command | a console command runs |

## Events (reactions)

`event = "…"` of `[[map.<Map>.when]]` (see [reactions.md](../reactions.md)).

| Word | Meaning |
|---|---|
| start | XIII can move: the level has started and its opening scene is over |
| near | a player comes within distance (300 by default) of a character; not during a cut-scene |
| enter | a player comes within radius (300 by default) of a point on the ground |
| sees | a character sees a player, with its own eyes |
| death | a character dies (who killed it is not known) |
| damage | a character loses health |
| fire | each shot of a player's gun (without who), of who, or of every character (who = "all"): actor is who fired, class the weapon's |
| pickup | an item comes into a player's inventory |
| event | the level fires an event (a trigger, a door, a scene: events.md of the names) |
| key | a player presses a keyboard key (F1-F12, A-Z, 0-9) |
| after | once, that many seconds after XIII can move (after the opening scene) |
| every | again and again, every that many seconds once XIII can move |

## Actions (reactions)

Each item of `then = [ … ]` of `[[map.<Map>.when]]`: `{ <action> = … }`.

| Word | Meaning |
|---|---|
| say | a character says a line; by: who says it (the event's character, else the player) |
| message | shows a message on the screen of every player |
| objective | shows a new goal, as the game's goals |
| objective_done | shows a goal as reached |
| give | gives a weapon or an item; to: who gets it (the player), who keeps what he holds |
| spawn | makes a character or an object appear, standing on position; team = "enemy" makes one that attacks |
| remove | takes an actor out of the level |
| change | changes a character now, with the settings of [[map.<Map>.change]] |
| trigger | fires an event of the level: opens its doors, starts its movers and scenes |
| sound | plays a sound; at: an actor or a position (the player) |
| music | plays a music of the game |
| next_level | ends the mission as won and goes on to that map |
| fail | ends the mission as lost, with that text |
| log | writes a line in mods-log.txt |
| camera | a scene: every player's view goes there, looking at look_at, for seconds |
| camera_back | ends a scene: the players' own view comes back |
| go_to | a story character walks (run = true: runs) there |
| follow | a story character follows an actor |
| attack | a soldier takes that actor as its enemy |
| look_at | a character turns to look at it |
| stop | stops a go_to, a follow or a patrol |
| play | a character plays an animation |
| restart | a mods character's state look (its animation and its sound) starts again from the beginning |
| teleport | moves an actor there at once |
| kill | the character dies, as from a gun shot |
| behave | a character runs a behaviour defined by the mod's scripts |
| patrol | a character walks from point to point, again and again |

## Key names

The setting `key = "…"` of a `[[shortcut]]` block (the cheat build of the game already uses F5, F6, F7 and F11).

`F1`, `F2`, `F3`, `F4`, `F5`, `F6`, `F7`, `F8`, `F9`, `F10`, `F11`, `F12`, `A … Z`, `0 … 9`, `NumPad0 … NumPad9`, `Insert`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`

## Weapon fire modes

`WeaponMode = "…"` in a weapon's class table.

| Word | Meaning |
|---|---|
| WM_Auto | fires while the button is held |
| WM_SemiAuto | one shot per press |
| WM_Burst | three shots per press |

## How a weapon is held

`FiringMode = "…"` in a weapon's class table (how a character holds and fires it).

| Word | Meaning |
|---|---|
| FM_1H | one hand (9mm, magnum, mini Uzi) |
| FM_2H | two hands (hunting rifle, pump shotgun, crossbows, harpoon) |
| FM_M16 | assault rifle (M16, Kalashnikov) |
| FM_2HHeavy | heavy machine gun (M60) |
| FM_Bazook | bazooka |
| FM_Snipe | sniper rifle |
| FM_Throw | thrown (grenades, knives, darts) |
| FM_Fists | fists |
| FM_Stun | objects (chairs, bottles) |
| FM_BazookAlt, FM_ARAlt, FM_ShotGunAlt, FM_HarpoonAlt, FM_44Alt | alternate fire |

## Console commands

Press F2 in the game, type, Enter.

| Word | Meaning |
|---|---|
| modwho | names the character you look at, with an entry to paste |
| modpos | your position and facing, for `position` and `yaw` |
| modkey <key> | does what the keyboard shortcut of that key does |

## Settings of the simple blocks

Settings of a character: `look`, `skin`, `health`, `name`, `voice`, `weapon`, `team`,
`behaviour`, `can_die`. Added actors: `class`, `model`, `character`, `position`, `yaw`, `rotation`,
`scale`, `collision`, `properties`, `from_checkpoint`. Changes: `who`, `near`, `radius`.
Dialogues: `by`, `line`, `text`, `sound`, `when`, `distance`, `event`, `delay`, `once`.
Reactions: `event`, `who`, `distance`, `position`, `radius`, `name`, `key`, `seconds`, `once`, `then`,
`run`, `from_checkpoint`; in `then`: `by`, `to`, `at`, `look_at`, `run`, `loop`, `yaw`, `team`.
