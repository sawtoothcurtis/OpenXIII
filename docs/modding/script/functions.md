# All the functions

Made from the game's own list of script functions (a test checks it is up to date). The other pages show
how to use them: [scripts](README.md).

## Events: on(event, ..., fn)

| Event | Then | fn receives | When |
|---|---|---|---|
| `"start"` | - | `player` | when the map has started and a player is there |
| `"near"` | `who, distance` | `player, actor` | when a player comes within distance (300 by default) of who; not during a cut-scene |
| `"enter"` | `position, radius` | `player` | when a player comes within radius of a point |
| `"sees"` | `who` | `actor, player` | when who sees a player (with its own eyes) |
| `"death"` | `who` | `actor` | when who dies |
| `"damage"` | `who` | `actor, amount, by, damage_type, weapon` | when who loses health: by is who hit it (nil when the game does not say), damage_type the kind of damage ("XIII.DTGunned"), weapon the class of the weapon by held then |
| `"fire"` | `[who]` | `shooter, weapon_class` | at each shot of a player (without who), of who, or of every character ("all") |
| `"pickup"` | - | `player, class` | when an item comes into a player's inventory |
| `"event"` | `name` | `player` | when the map fires that event (a trigger, a door, a scene); the player who caused it |
| `"key"` | `key` | `player` | when a player presses the key (A-Z, 0-9, F1, F2, F10, F12...; the game window keeps F3 to F9 and F11) |
| `"map_end"` | - | `map` | when the map is left for another one (a new level, a checkpoint loaded, the menus) |
| `"loading"` | - | `from_map, to_map, failed` | when the next map starts loading, after map_end; failed is "failed" after a lost mission. What it draws with hud_... is the loading screen, and hold_start() keeps it until release_start() |
| `"loaded"` | - | `map` | while the start is held: the next map is ready (the handler is the new map's script) |
| `"loading_key"` | - | - | while the start is held: at each press of a key, a button or the screen |

`on` returns a number that `cancel` takes.

## Functions

| Function | What it does | Example |
|---|---|---|
| `on(event, ..., fn)` | Runs fn when something happens: "start" (fn(player)), "near" who, distance (fn(player, actor)), "enter" position, radius (fn(player)), "sees" who (fn(actor, player): the character's own sight, CanSee), "death" who (fn(actor)), "damage" who (fn(actor, amount, by, damage_type, weapon)), "fire" [who] (fn(shooter, weapon_class)), "pickup" (fn(player, class)), "event" name (fn(player): the player who caused it when the game says, else the main player), "key" key (fn(player)), "map_end", "loading", "loaded", "loading_key" (the change of map: see the events). Returns an id for cancel. | `on("near", "Visitor", 250, function(player, visitor) say(visitor, "Hello.") end)` |
| `wait(seconds)` | Pauses the handler for that many seconds of game time. | `wait(2.5)` |
| `wait_until(fn[, seconds])` | Pauses the handler until fn() returns true (checked each frame), or for at most that many seconds. Returns true if fn did. | `wait_until(function() return not guard.alive end, 60)` |
| `has_control()` | True when a player can move: no cut-scene is playing. The first frames of a level come before its opening scene: wait a moment first. | `wait(1) wait_until(has_control)` |
| `after(seconds, fn)` | Runs fn once, that many seconds of game time from now. Returns an id for cancel. | `after(10, function() message("Ten seconds.") end)` |
| `every(seconds, fn)` | Runs fn every that many seconds of game time; fn returning false stops it. Returns an id for cancel. | `every(1, function() log(player().health) end)` |
| `cancel(id)` | Stops a handler of on, after or every. | `local id = every(1, tick) cancel(id)` |
| `map()` | The name of the map being played ("Plage01"). | `if map() ~= "Plage01" then return end` |
| `find(who)` | The actor named who (its name, then its Tag, then its PawnName; a mods [character] by its name first), or nil. | `local pam = find("Pam")` |
| `find_all(who)` | Every actor named who, as a list: all the mods [character]s of that name on the map, alive or dead; else every actor alive that find would give. | `for _, s in ipairs(find_all("Singer")) do s:restart() end` |
| `player()` | The player (the local one in a split screen game), or nil. | `local p = player()` |
| `players()` | Every player of the game, as a list. | `for _, p in ipairs(players()) do p.health = 100 end` |
| `spawn({character= or class=, position= or near=, yaw=, team=, behaviour=})` | Creates a mods [character] or an actor of a class, standing on position (a point on the ground), or near an actor or a point (the first spot around it with room and ground). Returns the actor, or nil. | `spawn{character = "Guard", position = {-470, -860, 1190}, yaw = 180, team = "enemy"}` |
| `remove(who)` | Takes an actor out of the game. Returns true if it was removed. | `remove("MedPick0")` |
| `say(who, line)` | Who says a line: a line of the game by its name, your own subtitle, or {line=, text=, sound=} as in [[map.<Map>.say]]. | `say("Pam", "Plage01_Pam_03")` |
| `message(text[, player])` | Shows a message on the screen of every player, or of that player only. | `message("The guard is coming.")` |
| `log(text)` | Writes a line in mods-log.txt. | `log("scene started")` |
| `change(who, {look=, health=, team=, can_die=, weapon=, name=, voice=, skin=, properties=})` | Changes an actor now, with the same words as the toml's [[map.<Map>.change]]. Returns how many actors changed. | `change("Pam", {look = "Jones", health = 300})` |
| `objective(text[, player])` | Shows a new goal on the screen, as the game's goals are shown (to that player only if given). | `objective("Hold the beach")` |
| `objective_done(text[, player])` | Shows a goal as reached (to that player only if given). | `objective_done("Hold the beach")` |
| `trigger(event[, who])` | Fires an event of the map (opens its doors, starts its movers and scenes), caused by who (the player by default). | `trigger("PorteCabane")` |
| `sound(name[, actor or position])` | Plays a sound of the game or of the mod, from the player, an actor or a position. Returns an id for stop_sound. | `local alarm = sound("Mod.Alarm", {-470, -860, 1190})` |
| `music(name)` | Plays a music of the game (a sound of its music banks). | `music("XIIIMusic.Combat.hCombat1")` |
| `next_level([map])` | Ends the mission as won and goes on to that map ("Banque01", "Banque01#Teleporter", or a mod's map); without a map, the next one of campaign{...}. | `next_level("Banque01")` |
| `campaign({map, map, ...})` | Lists the maps of the mod's campaign in order: next_level() then goes to the next one. Write it in every map's script (a script starts again with each map). | `campaign{"BeachNight", "HutAtDawn"}` |
| `fail(reason)` | Ends the mission as lost, with that text on the game over screen. | `fail("The guard escaped.")` |
| `random([m[, n]])` | A random number of the scripts' own (never the game's): random() between 0 and 1, random(n) from 1 to n, random(m, n) from m to n. | `local wave = random(3, 6)` |
| `class_get(class, property)` | The default value of a property of a class. | `log(class_get("XIII.Beretta", "ReloadCount"))` |
| `class_set(class, property, value)` | Changes the default value of a property of a class (for the actors created afterwards). | `class_set("XIII.Beretta", "ReloadCount", 20)` |
| `behaviour(name, fn)` | Defines a behaviour: fn(self) drives a character as a handler does (orders, wait, wait_until), until it dies or behaves otherwise. | `behaviour("Bodyguard", function(self) while true do self:follow(player()) wait(1) end end)` |
| `actors_near(position or actor, radius[, class])` | The actors of the level within radius (the scenery's static meshes, its doors and objects, the characters alive or dead; players not included), of that class if given, nearest first. | `for _, a in ipairs(actors_near(player(), 500, "StaticMeshActor")) do log(a.name) end` |
| `char_area(position, radius[, amount])` | The level's own walls, floors and ceilings, and the pieces of scenery too big to be charred as a whole, within radius are drawn charred: amount from 0 to 1 (1 by default; the darkest stays), less towards the edge. A surface drawn in triangles wider than radius is left as it is (a darkened triangle would show far from the fire). It stays as long as the level. | `char_area({-470, -860, 1191}, 300)` |
| `characters_near(position or actor, radius)` | The characters alive within radius (players not included), nearest first. | `for _, c in ipairs(characters_near(player(), 1000)) do log(c.name) end` |
| `camera(position, look_at[, seconds] or {from=, look_at=, seconds=, fov=, follow=, path=, orbit=})` | A scene: every player's view goes to position, looking at an actor (followed with the eyes) or a position, with the cinematic frame and the controls frozen; back after seconds if given. With a table: fov = the field of view; follow = an actor the camera moves with; path = {point, ...} the camera goes along from `from` over the seconds; orbit = {radius=, height=, turns=} the camera turns around what it looks at over the seconds. | `camera{from = {-300, -900, 1300}, look_at = find("Visitor"), orbit = {turns = 0.5}, seconds = 6}` |
| `checkpoint(description[, teleporter])` | The levels' save point: the game saves here, with that description, and takes up again at the map's teleporter given (one of the map's, as the catalog lists them) or at the map's start. The mod's save table is saved with it. | `checkpoint("On the beach")` |
| `focus({on=, seconds=, zoom=, zoom_fov=, zoom_distance=, vignette=, danger=})` | The levels' "sixth sense" inset: a window on every player's screen that shows an actor (or a point), as the levels show a clue or a hidden enemy; zoom = true zooms on it, danger = true is the red danger inset. | `focus{on = find("Visitor"), seconds = 3, zoom = true}` |
| `slow_motion(factor[, seconds])` | The game runs factor times as fast (0.5: half speed; 1: normal), for that many seconds if given; slow_motion(1) ends it. | `slow_motion(0.4, 3)` |
| `fade({to=, seconds=, player=} or {back = true, seconds=, player=})` | Every player's view (or that player's only) fades to a colour ("black", "white", "red" or {r, g, b} from 0 to 255) over the seconds, and back with back = true. | `fade{to = "black", seconds = 2}` |
| `time()` | Seconds of game time since the level started: the clock of wait and every, the same at any frame rate. | `local t0 = time()` |
| `trace(from, to[, by])` | What the line from one point to another meets first (the level or an actor), as {position=, normal=, actor=, material=} (actor is nil for the level itself; material is the name of the surface's texture or shader), or nil when nothing is in the way. The line starts from actor by (the player by default), which it never meets. | `local hit = trace(p.position, {0, 0, 0}) if hit then log(hit.position[3]) end` |
| `heat({at=, radius=, seconds=, strength=, name=})` | Heat shimmer: the view waves a little over a round area around at (a position, or an actor it follows) of that radius, for that many seconds (strength from 0 to 2, 1 by default). A new heat of the same name replaces the earlier one. | `heat{at = find("Visitor"), radius = 120, seconds = 5}` |
| `effect({class=, position=, rotation= or facing=, on=, seconds=})` | Puts an actor of a class (an emitter, a projector, a light...) exactly at position (no search for the ground), turned by rotation = {pitch, yaw, roll} in degrees or facing = a direction {x, y, z}; on = an actor that carries it (it moves with it); seconds = how long it stays. Returns the actor, or nil. | `effect{class = "XIII.GrenadBlast", position = {-470, -860, 1191}, facing = {0, 0, -1}}` |
| `camera_back()` | Ends a scene: the players' own view and controls come back. | `camera_back()` |
| `panel({from=, look_at=, fov=, seconds=, after=, x=, y=, size=, appear=, sound=, text=, word=, see_player=} or a list of them)` | Comic-book insets as the levels show them: each one a small window with its own camera (from a point, looking at an actor or a point), on every player's screen, one after the other (after = seconds before the next one). The screen is 640x480; size up to 256 (insets shown together share a 256x256 picture: four of 128, the levels' size); appear: none, left, above, stretch, stretch_up, zoom, right, below, top_left, top_right, bottom_left, bottom_right; sound: none, tip, warning, dialog; word: an onomatopoeia picture of the levels (XIIIvignettes). Returns the seconds until the last one closes. | `panel{from = {-300, -900, 1300}, look_at = find("Visitor"), seconds = 3, appear = "zoom"}` |
| `game()` | What is being played: {difficulty =, language =, mode =}: the difficulty (0 to 3), the language of the game's texts ("int", "frt", "det", "itt", "est"), and "solo", "split" (players on one screen) or "network". | `if game().mode == "network" then log("network game") end` |
| `preload(name)` | In the script's main code or its on("start") handler only: asks for an object ("Package.Name": a mesh, a texture, a sound) to be made ready before the level starts, as preload = [...] in the mod.toml does. | `preload("XIIIPersos.SpadsM")` |
| `stop_sound(id)` | Stops a sound that sound() played (sound() gives its id). Returns false when it was stopped already. | `local id = sound("XIIISound.Ambient.SancFire", find("Hut")) after(5, function() stop_sound(id) end)` |
| `music_stop()` | Stops the music. | `music_stop()` |
| `hud_text({id=, text=, x=, y=, size=, font=, colour=, align=, player=})` | Keeps a text on the screen until hud_remove: x and y from 0 to 1 of the view (0, 0 the top left corner), size "small" or "medium" (or font = a font of the game), colour = a name or {r, g, b[, a]} (0 to 255), align "left", "centre" or "right" (what x is), player = only on that player's view. Returns its id (the one given, or a new one); an id already shown is replaced. | `hud_text{id = "goal", text = "Find the key", x = 0.5, y = 0.1, align = "centre", colour = "yellow"}` |
| `hud_counter({id=, label=, value=, x=, y=, ...})` | A text showing label then value (a number), as hud_text; hud_update(id, {value = n}) changes it. | `hud_counter{id = "kills", label = "Enemies: ", value = 0, x = 0.05, y = 0.9}` |
| `hud_timer({id=, label=, seconds=, x=, y=, ...})` | A text counting down from seconds (game time), as minutes:seconds after label, as hud_text; it stops at 0:00. | `hud_timer{id = "bomb", label = "Bomb: ", seconds = 90, x = 0.8, y = 0.05}` |
| `hud_bar({id=, x=, y=, w=, h=, value=, colour=, back=, player=})` | A bar w wide and h high (0 to 1 of the view), filled from the left to value (0 to 1) with colour, over back if given. | `hud_bar{id = "boss", x = 0.3, y = 0.05, w = 0.4, h = 0.02, value = 1, colour = "red", back = {40, 40, 40}}` |
| `hud_image({id=, texture=, x=, y=, w=, h=, colour=, player=})` | A texture of the game or of the mod stretched over w by h (0 to 1 of the view). | `hud_image{id = "icon", texture = "Mod.FlamethrowerIcon", x = 0.9, y = 0.8, w = 0.08, h = 0.1}` |
| `hud_line({id=, from=, to=, colour=, player=})` | A line from {x, y} to {x, y} (0 to 1 of the view). | `hud_line{id = "cross", from = {0.45, 0.5}, to = {0.55, 0.5}, colour = "green"}` |
| `hud_update(id, {settings})` | Changes what an item of the screen shows (the same settings as when it was made). Returns false when there is no such item. | `hud_update("kills", {value = 3})` |
| `hud_remove([id])` | Takes an item off the screen; without an id, every item of the mod's scripts. | `hud_remove("goal")` |
| `text_size(text[, size or font])` | The width and height of a line of text, as parts of a 640x480 screen (multiply by 640 and 480 for its pixels there). | `local w, h = text_size("Hello", "small")` |
| `text(section, key[, file])` | A text of the game's language files in the player's language (file "Engine" by default), or nil. | `local recap = text("EndMsgMap", "Banque01")` |
| `hold_start()` | In an on("loading") handler: the next level does not start until release_start(); the loading screen shows the hud_... items the loading handlers made. | `on("loading", function() hud_text{text = "...", x = 0.5, y = 0.5} hold_start() end)` |
| `release_start()` | Lets the held level start (at once when it is ready, otherwise as soon as it is, behind the game's loading animation). | `on("loading_key", function() release_start() end)` |

## Actors

What `find`, `spawn`, `player` and the events give is an actor.

| Field | Can be changed | What it is |
|---|---|---|
| `actor.name` | no | its name in the map ("Cine0", "MedPick2") |
| `actor.class` | no | its class ("XIDPawn.BaseSoldier") |
| `actor.tag` | no | its Tag |
| `actor.alive` | no | false once it is dead or removed |
| `actor.health` | yes | its health (characters); above 0 |
| `actor.position` | yes | {x, y, z}: the point on the ground where it stands |
| `actor.yaw` | yes | the direction it faces, in degrees (0 faces +X, 90 faces +Y) |
| `actor.team` | yes | "friend", "neutral" or "enemy": how a character treats the players (nil: the game's own) |
| `actor.weapon` | no | the class of the weapon in its hands (give() gives another) |
| `actor.charred` | yes | how black it is drawn, from 0 (as the game draws it) to 1 (black): burnt characters and objects. A character's worn pieces (vest, helmet, beret) go with it, never its weapons |
| `actor.size` | no | its widest side as it is drawn, in units (a man is about 170 tall): a door, a crate or a whole hall |
| `actor.ammo` | yes | the ammunition of the weapon in a character's hands (or of a weapon), nil without one |
| `actor.muzzle` | no | {x, y, z}: on a weapon held with a FirstPersonModel (p:get("Weapon").muzzle), the front end of that model as its owner sees it in his hands, in first person: the point of the world his view shows there (an effect put there is seen on the model); nil otherwise (no such model, or not drawn on this computer) |
| `actor.ground` | no | what is under a character's feet: the name of the level's footsteps there ("PNJFSNei" or "PNJFSNei__hPNJFSNei": snow, Met: metal, Hrb: grass...), or nil |

| Order | What it does | Example |
|---|---|---|
| `actor:go_to(target[, "run"])` | A story character (Cine2) walks, or runs, to an actor or a position. Soldiers follow their own AI. | `janet:go_to({-470, -860, 1190})` |
| `actor:follow(actor[, {catch_up = distance}])` | The character follows an actor until stop(), finding its way through doors; with catch_up, when it is further than that and no player sees it, it is put back near its leader. | `guard:follow(player(), {catch_up = 1500})` |
| `actor:attack(actor)` | A soldier takes this actor as its enemy (a player: it becomes hostile to the players). | `guard:attack(player())` |
| `actor:look_at(actor or position)` | The character turns to look at it. | `janet:look_at(player())` |
| `actor:stop()` | Stops a go_to or a follow. | `janet:stop()` |
| `actor:play(animation[, loop])` | Plays an animation of the character's model (animations.md). | `janet:play("WaitNeutre", true)` |
| `actor:restart()` | A mods [character] starts the look of its state again from the beginning: its animation and its sound ([character.<Name>.state.<State>]). Returns false for any other actor. | `singer:restart()` |
| `actor:give(weapon or class)` | Gives a weapon (a name of the toml's weapon list, or a class) or an item. | `player():give("Shotgun")` |
| `actor:teleport(actor or position)` | Moves the actor there at once. Returns false if there is no room. | `guard:teleport({-470, -860, 1190})` |
| `actor:kill()` | The character dies, as from a gun shot. | `guard:kill()` |
| `actor:get(property)` | The value of any property of the actor (characters.md, weapons.md); "Emitters[0].Disabled" reads a property of an object it holds. | `log(guard:get("GroundSpeed"))` |
| `actor:set(property, value)` | Writes any property of the actor (or of an object it holds: "Emitters[0].Disabled"); a wrong type gives a clear error. | `guard:set("GroundSpeed", 300)` |
| `actor:call(function, ...)` | Calls a function of the actor's class, as the game's own scripts call it. | `door:call("Trigger", player(), player())` |
| `actor:cine(line)` | For experts: one command line of the levels' scene language, given to a story character as the game's own scenes give it (CineController2): "Anim Once Rire", "LookAt XIIIPlayerPawn0", "Shoot", "Give XIII.Beretta"... An unknown command is ignored by the game, as in the levels. | `visitor:cine("Anim Once Rire")` |
| `actor:retexture(from, to)` | Draws the actor (a static mesh of the scenery: a hut, a crate) with texture to where its own is from (the names trace() gives as material); to = "" draws it as it was. Only this actor changes. | `hut:retexture("XIIIPlage.PLwood05", "XIIICamp.CSbrule")` |
| `actor:pin_to(other[, {forward, right, up}])` | The actor (an effect) is kept every frame at that point of the other actor's own frame, until pin_to(nil): a flame at the tip of a gun a character holds (its ThirdPersonActor, placed as the character is posed). In a network game each player sees it on the other actor as his own screen draws it; its rotation stays the script's, sent relative to the view of the character holding it. | `jet:pin_to(p:get("Weapon"):get("ThirdPersonActor"), {100, 0, 3})` |
| `actor:behave(name or fn or nil)` | The character runs a behaviour (one defined with behaviour(), or a function); nil stops it. | `guard:behave("Bodyguard")` |
| `actor:patrol({points...}[, "run"])` | The character walks from point to point, again and again, until stop() or another order. | `guard:patrol{{-470, -860, 1190}, {-200, -860, 1190}}` |
| `actor:is_a(class)` | True when the actor is of that class or of a class made from it: a full name ("XIII.Beretta") or a class's own name ("Pawn"). | `if a:is_a("Pawn") then log(a.name, "is a character") end` |
| `actor:trigger([by])` | Triggers the actor (a door, a mover, an emitter: its Trigger), caused by by (the player by default). | `find("PorteCabane"):trigger()` |
| `actor:has(weapon or class)` | True when the character carries that weapon (a name of the toml's weapon list, or a class) or item. | `if player():has("Shotgun") then message("You have the shotgun.") end` |
| `actor:hurt(amount[, by[, damage_type[, at[, push]]]])` | The actor takes damage as from the game's weapons (its TakeDamage: the game decides how much, with the difficulty and armour): caused by by, of damage_type ("XIII.DTGunned" by default; "XIII.DTFire", "XIII.DTBite"...), hit at a point (its middle by default) and pushed by push {x, y, z}. Returns the health a character really lost. | `find("Guard"):hurt(25, player(), "XIII.DTFire")` |
| `actor:can_see(actor)` | True when the character sees that actor with its own eyes (walls hide it); false for an actor without eyes (no controller). | `if dog:can_see(player()) then dog:attack(player()) end` |
| `actor:take(weapon or class)` | Takes that weapon or item away from the character. Returns how many were taken. | `player():take("XIII.Beretta")` |
| `actor:aim()` | Where the character looks: {from =, direction =, hit =, actor =}: the point of its eyes, the direction of its view (where a player aims), the first point the view meets (nil when nothing is within 20000 units) and the actor there (nil for the level itself). | `local a = player():aim() if a.hit then effect{class = "XIII.GrenadBlast", position = a.hit} end` |

## In a network game

Scripts run on the host only; `players()` lists the guests' characters too. What the scripts show is sent to the
guests: the actors made by `effect{}` appear on every guest, and what a script does to them afterwards follows them:
properties written with `set`, functions called with `call` (except `SetOwner` and `SetBase`, which the game sends
itself), sounds played from them, and their moves (`SetLocation`, `SetRotation`, at most ten times a second). An
effect that only its owner sees (`bOnlyOwnerSee`: a gun's flame in first person) is placed again at the guest's own
view every frame, without delay; one owned by a character follows that character. `charred`, `char_area`,
`retexture` and `heat` are sent as well. A guest who joins late gets the charred actors and areas, the textures and
every effect still there, as they are now. The mods' log tells once a minute what this costs.

A guest's fire button is not seen on the host (`bFire` stays 0 there): the guest's gun is fired by the host shot
after shot (its ammunition goes down), and the weapon's `StopFiringTime` changes when the guest lets go of an
automatic weapon's button.

## Lua

Scripts are Lua 5.4. They can use `string`, `table`, `math` and `coroutine` (each mod has its own copy),
and the base functions without the ones that read files or load code. `save` is the mod's memory
(numbers, strings, true/false and tables of them).
