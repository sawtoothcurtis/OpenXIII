# Make something happen when…

When XIII comes near someone, when a character dies, when you press a key, some seconds
after the start: something happens. A `when` block says it in a few lines of the mod's `.toml`
file, without any script.

## Recipe

```toml
[[map.Plage01.when]]
event = "start"                    # when XIII can move (after the opening scene)…
then = [
  { message = "Someone waits for you at the hut." },
  { spawn = "Carrington", position = [-470, -860, 1190] },   # …General Carrington appears
]

[[map.Plage01.when]]
event = "near"                     # when XIII comes near…
who = "Carrington"                 # …the general
distance = 150                     # within 150 units (optional)
once = true                        # only the first time (optional)
then = [
  { say = "Hual02_Carrington_06" },   # he says a line of the game
  { give = "Shotgun" },               # XIII gets a shotgun
]
```

Read it aloud: *at the start, a message and Carrington at the hut; when XIII is near him, once,
then he says his line and XIII gets a shotgun.* The actions run in the order of the list.
Another one, with a key and an enemy:

```toml
[[map.Plage01.when]]
event = "key"
key = "L"
then = [ { spawn = "Guard", position = [-250, -900, 1190], team = "enemy", weapon = "Pistol" } ]

[[map.Plage01.when]]
event = "death"
who = "Guard"
then = [ { message = "The guard is down." } ]
```

Names you can use here: [names/characters.md](names/characters.md) (characters),
[names/weapons.md](names/weapons.md) (weapons), [names/words.md](names/words.md) (the events and
actions below), and on your PC `Mods/_Guide/names/maps.md` (the characters each map places, for
`who`), `Mods/_Guide/names/dialogues.md` (lines for `say`) and `Mods/_Guide/names/events.md` (the
events of each map).

## Events

`event = "…"` and its own settings:

| Event | Its settings | When | Gives |
|---|---|---|---|
| `start` | - | XIII can move: the level has started and its opening scene is over | the player |
| `near` | `who`, `distance` (300) | XIII comes within distance of a character (not during a cut-scene) | the player, the character |
| `enter` | `position`, `radius` (300) | XIII comes within radius of a point on the ground | the player |
| `sees` | `who` | a character sees XIII, with its own eyes | the character, the player |
| `death` | `who` | a character dies (who killed it is not known) | the character |
| `damage` | `who` | a character loses health | the character, the amount lost |
| `fire` | `who` (the players) | each shot of a player's gun, of who, or of every character (`who = "all"`) | who fired (actor), the weapon's class |
| `pickup` | - | an item comes into XIII's inventory | the player, the item's class |
| `event` | `name` | the level fires one of its events (a trigger, a door, a scene) | the player |
| `key` | `key` | you press a keyboard key (F1-F12, A-Z, 0-9) | the player |
| `after` | `seconds` | once, that many seconds after XIII can move | - |
| `every` | `seconds` | again and again, every that many seconds once XIII can move | - |

`who` is a character of the level: its name in `Mods/_Guide/names/maps.md` (`who` column), or a
character your mod made appear (`spawn = "Guard"` is then found with `who = "Guard"`). It must be
in the level when XIII can move: on Plage01, Janet (Pam) is only there during the opening scene.

A script's `on("start")` runs at once, during the opening scene; a block's `start`, `after` and
`every` wait until XIII can move, so that its messages are seen.

## Actions

Each item of `then = [ … ]` is one action: its word with its value, and its own settings.

| Action | Its value | Its settings | Example |
|---|---|---|---|
| `say` | a line of the game, or your own subtitle | `by` | `{ say = "Plage01_Pam_03", by = "Pam" }` |
| `message` | a text | - | `{ message = "The guard is coming." }` |
| `objective` | a text | - | `{ objective = "Hold the beach" }` |
| `objective_done` | a text | - | `{ objective_done = "Hold the beach" }` |
| `give` | a weapon or an item | `to` | `{ give = "Shotgun" }` |
| `spawn` | a character or a class | `position`, `yaw`, and a character's settings: `team`, `behaviour`, `weapon`, `health`, `look`, `skin`, `name`, `voice`, `can_die` | `{ spawn = "Guard", position = [-250, -900, 1190], team = "enemy", weapon = "Pistol" }` |
| `remove` | who | - | `{ remove = "MedPick0" }` |
| `change` | who | `look`, `health`, `team`, `can_die`, `weapon`, `name`, `voice`, `skin`, `properties` | `{ change = "actor", look = "Jones", health = 300 }` |
| `trigger` | an event of the level | `by` | `{ trigger = "PorteCabane" }` |
| `sound` | a sound, or `Mod.<Name>` | `at` | `{ sound = "Mod.Alarm", at = "actor" }` |
| `music` | a music of the game | - | `{ music = "XIIIMusic.Combat.hCombat1" }` |
| `next_level` | a map, or `true` for the next map of your campaign | - | `{ next_level = "Banque01" }` |
| `fail` | a text | - | `{ fail = "The guard escaped." }` |
| `log` | a text | - | `{ log = "Janet was reached" }` |
| `camera` | a position | `look_at`, `seconds` | `{ camera = [-300, -900, 1300], look_at = "actor", seconds = 4 }` |
| `camera_back` | `true` | - | `{ camera_back = true }` |
| `go_to` | an actor or a position | `who`, `run` | `{ go_to = [-470, -860, 1190], who = "Pam" }` |
| `follow` | an actor | `who` | `{ follow = "player", who = "Pam" }` |
| `attack` | an actor | `who` | `{ attack = "player", who = "Guard" }` |
| `look_at` | an actor or a position | `who` | `{ look_at = "player", who = "Pam" }` |
| `stop` | who | - | `{ stop = "Pam" }` |
| `play` | an animation | `who`, `loop` | `{ play = "WaitNeutre", who = "Pam", loop = true }` |
| `restart` | who | - | `{ restart = "every Singer" }` |
| `teleport` | an actor or a position | `who` | `{ teleport = [-470, -860, 1190], who = "Guard" }` |
| `kill` | who | - | `{ kill = "actor" }` |
| `behave` | a behaviour of your scripts | `who` | `{ behave = "Bodyguard", who = "actor" }` |
| `patrol` | a list of positions | `who`, `run` | `{ patrol = [[-470, -860, 1190], [-200, -860, 1190]], who = "Guard" }` |

`restart` starts the look of a mod character's state again from its beginning: its animation
and its sound (`[character.<Name>.state.<State>]`).

`"every <Name>"` as who (in `remove`, `change`, `restart` and the orders: `stop`, `kill`,
`play`...) does the action for each character of the mod of that name: `{ remove = "every
Singer" }` takes them all away, the dead ones too; the orders go to the living ones.

Four words stand for what the event gave: `"player"` (XIII), `"actor"` (the character of the
event: Pam for `near`, the dead one for `death`), `"amount"` (`damage`) and `"class"`
(`pickup`). `say` is said by the event's character, else by XIII, unless `by` says who; the
orders (`go_to`, `follow`, `play`…) go to the event's character unless `who` says who; `give`
goes to XIII unless `to` says who: the weapon goes in his inventory and he keeps what he holds
(the weapon keys or the mouse wheel take it). A character made with `team = "enemy"` is a soldier
that comes for XIII at once; give it a `weapon`.

Other settings of the block: `once = true` (only the first time), `from_checkpoint = 2` (only
when the level starts from that checkpoint or a later one), and `run = "my_function"`: a
function of the mod's own scripts, called after the actions with what the event gave (see
[script/](script/README.md)).

A long list can also be written one action per block, which reads the same:

```toml
[[map.Plage01.when]]
event = "start"

[[map.Plage01.when.then]]
message = "Welcome to the beach."

[[map.Plage01.when.then]]
give = "Shotgun"
```

In a network game or in co-op, the blocks run on the host only (the one who started the
game), as the game's own events; everybody sees what they do.

## If it does not work

- Open `mods-log.txt` (next to the Mods folder): each block is listed as the game read it
  ("when near Pam then say, give on Plage01"), and "when rule fired" is written each time
  one runs.
- A wrong word skips the whole block with one line that names the closest words: `unknown event
  "neer" (did you mean near?)`, `no action in { mesage } (did you mean message?)`,
  `unknown setting bye for say (did you mean by?)`.
- `"actor"` needs an event that gives a character (`near`, `sees`, `death`, `damage`).
- `near` does nothing during a cut-scene; `who` must be a character that is in the level at
  that moment (on Plage01, Janet dies in the opening scene).
- A key does nothing: the game window keeps F3, F4, F8 and F9 (and F5, F6, F7, F11 in the test
  builds), and the game's own settings use others (F1, F12, W, A, S, D…); `mods-log.txt` says so
  when the game starts. Free keys: K, L, J, H, U, O, P, Insert, NumPad2 to NumPad9.
- A given weapon is not in XIII's hands: it is in his inventory; press the weapon keys.
- The other problems: [troubleshooting.md](troubleshooting.md).

More (wait, loops, your own conditions): [scripts](script/README.md). A block copies into a
script almost word for word: `on("near", "Pam", 300, function(player, actor) say(actor,
"Plage01_Pam_03") end)`.
