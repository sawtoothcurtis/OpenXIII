# Content from any level, and network games

Packages loaded when a mod names their content, and what a mod changes in a network game. Back to [the modding guide](../README.md).

## Content from any level

Textures, meshes and other objects are named `Package.Group.Name` (`Texture'…'` or
`StaticMesh'…'` forms work too). A mod can name any of them in a value: the property of a
class, of an added actor or of a `[character]` (`Mesh`, `StaticMesh`, `Skins[0]`…). The
object is looked up in the game's packages, and its package is loaded if it is not yet, as the
game's own `DynamicLoadObject` does. A decoration from another level can be added with
`[[map.<Map>.add]]` and a `StaticMesh` property. A wrong name gives one warning line and the
value is ignored.

`load_packages = ["…"]` (at the top of a mod file, or in `[map.<Map>]`) loads packages when a
level starts, for content no mod value names.

`content.md` lists the content packages (textures, static meshes, characters'
meshes and animations) with every object's name.

## Mods in network games

A server and its players must have the same mods: the network game designates the objects
they share (classes, sounds, models) by their rank in a list both sides make, and the mods'
new objects are in that list. So:

- **The server announces its mods.** With mods, a server (the dedicated server `oxiii server`
  or a game hosted from the menu) adds a package named `Mod` to the packages it announces. Its
  identity is made of each mod's folder name, version and **fingerprint** (a code computed from
  all the files of the mod's folder: changing any file changes it), in load order.
- **A player with other mods, or without mods, is refused** when joining, with a message that
  says why, shown by the game ("Rejected By Server"), for example
  `Your mods differ from the server's (blaster 1.0.0): mod blaster: version 1.0.1 instead of 1.0.0`
  or `This server uses mods (blaster 1.0.0): join it with OpenXIII (improved build) and the same mods`.
  The faithful build and the original game cannot join a server with mods. Copy the server's
  Mods folder to the players to have the same mods.
- **A player with mods can join a server without mods.** Its log warns that its mods then
  change its own game only.
- **What the mods add is shared like the game's own things**: actors of the mods' new classes
  (a new weapon on the ground, in a player's hands), the values of the classes (set on both
  sides by the same mods), the mods' sounds, textures and models, the animations of animated
  models. The map changes (`[[map.<Map>.add]]`, `remove`, `replace`) and the start weapons are
  made by the server only; the players receive the actors as the game sends them.
- **Skins.** The game sends a character's `Mesh`, `DrawScale`, `DrawScale3D`, `AmbientGlow`,
  `bUnlit` and `PrePivot` to the players, never its `Skins`. The skins a mod gives (`skin`, or
  `Skins` in `properties`, of `change`, `add`, `spawn` and `[character]`) are sent by the host to
  each player when the actor reaches them, in a `ClientMessage` whose text starts with
  `OXIIIMod.Skins.` (the channel of the actor, its class and its skins); the player writes them on
  that actor. Nothing is sent when no mod gives a skin. Split screen is one game: every view
  shows the same actors.
- **What scripts show.** Scripts run on the host only (`mod_net_fx.rs`). An actor made by
  `effect{}` on a network host gets `RemoteRole` SimulatedProxy and `bAlwaysRelevant`: the
  game's replication creates it on every player with its class and first place. What the game
  does not replicate goes in reliable `ClientMessage` calls whose text starts with
  `OXIIIMod.Fx|`, steps separated by `~` and fields by a backquote, each message at most 380
  bytes as the network writes it (a call must fit in one 512-byte packet, or the game drops it:
  "RPC bunch overflowed"):
  - on an effect actor (named by its channel on that connection, `e<channel>`): a property
    written (`s`, path and value), a function called (`c`; the player runs it as the host ran
    it, as authority, since an emitter's `Trigger` is not `simulated`), a sound played (`p`), a
    place (`m`: relative to its owner's view when only its owner sees it, then placed again at
    the player's own view every tick; at a point of another actor when a script pinned it there
    with `pin_to`, then placed again every tick on that actor as the player draws it (a flame at
    the tip of a gun in a character's hands), turned with the view of that character; relative
    to its owner character; or in the world). Places at a view or pinned go at most ten times a
    second per actor, only when they moved by 3 units or about 1.5 degrees; the others five
    times a second. The steps of an actor are kept while it lives: a player whose channel to it opens
    later gets them all, in order. An actor made with the number of one just taken out of the
    game starts with no record;
  - on the level (`w`): charred values of actors (`ch`, the actor named by channel `c<n>` or, for
    the map's own actors, by name `n<name>`), charred areas (`ar`), changed textures (`rt`) and
    heat shimmer (`h`). Charred values, areas and textures are sent to a player who joins late.

  Nothing is sent when no script makes an effect; the mods' log tells once a minute how many
  messages and bytes went to the players.

The log of both sides gives the mods' identities when the game starts, for example
`mods: network identity of mod blaster 1.0.0 (3F1C0D2E9A7B5C41)`.

Limits: a key's mod action (`[[action]]`) runs where the key is pressed: in a network game, a
player's spawn or `give` action only changes that player's own game.
