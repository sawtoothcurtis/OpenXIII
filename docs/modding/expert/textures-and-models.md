# Your own textures and 3D models

Textures, solid models and animated models a mod brings. Back to [the modding guide](../README.md).

## Your own textures

A mod can bring its own textures, as PNG files of the mod folder (any color type, sizes up to
4096×4096; the smaller mip levels are made from it):

```toml
[textures.Wall]                # a new texture "Mod.Wall", which also replaces a game texture
file = "textures/wall.png"
replace = "Package.Group.Texture"   # a texture name from content.md

[textures.Poster]              # a new texture only
file = "textures/poster.png"

[textures.Burn]                # a mark projected on walls (see Tracers, impacts and sparks)
file = "textures/burn.png"
clamp = true                   # not repeated: outside the image, its edge is read
```

- `replace` draws the mod's image wherever the game draws that texture: on characters and
  objects, on the level's surfaces, and inside the shaders and combined materials that use it.
  The names of the game's textures are in `content.md`.
- `Mod.<Name>` works wherever a texture is accepted in a value: a class value such as
  `CrossHair = "Mod.Poster"`, or the `properties` of an added actor or a `[character]`
  (`Skins[0] = "Mod.Poster"`). The game draws it as one of its own textures, with the size of
  the image; an image with transparent pixels is drawn with its transparency.
- The images are read once when the game starts; a missing or unreadable file gives one
  warning line and that texture is left out.

## Your own 3D models

A mod can bring its own static meshes (objects that do not bend: crates, weapons on the ground,
decorations), as glTF 2.0 files of the mod folder (`.gltf` with its `.bin` file, or `.glb`). A
model with a skeleton is an animated model (see [Animated models](#animated-models)):

```toml
[models.Crate]                     # a new static mesh "Mod.Crate"
file = "models/crate.gltf"
scale = 50                         # game units per glTF metre (optional, 50 by default)
replace = "StaticPlage2.Porte_lunch"  # also drawn instead of this game mesh (optional)
material = "XIIIPlage.PLsky02"     # draw every part with this game texture (optional)
collision = "box"                  # what blocks: "mesh" (default), "box" or "none" (optional)
outline = true                     # black comic-book outline (optional, true by default)
cartoon = true                     # flat comic-book shading (optional, true by default)
outline_width = 1.0                # outline thickness: 1.0 = the game's own, 2.0 = twice as thick
```

- `Mod.<Name>` works wherever a static mesh is accepted: the `StaticMesh` of an added actor (a
  pickup, a decoration…) or a class value.
- Each primitive of the glTF file becomes one part of the mesh. Its base colour texture, when it
  is a PNG file next to the model, is drawn on it; `material` draws a game texture instead.
- glTF is Y up, in metres, facing -Z; the game is Z up, facing +X: a model made facing forward
  in a 3D editor faces forward in the game. The model's origin is the actor's position.
- At most 65535 vertices and 65535 triangles in all, as the game's own meshes.
- The model is drawn in XIII's comic-book style, like the game's objects and characters: a black
  outline around it (as thick as on the game's objects) and flat shading. `outline = false` or
  `cartoon = false` turns either off (for a light shaft, a glow…). A part whose PNG texture has
  transparent pixels gets no outline, as in the game. `outline_width` makes the outline thicker
  or thinner (a number above 0 and up to 20; `1.0`, the default, is the thickness of the game's
  objects). The thickness on screen stays the same whatever the model's `scale`, its
  `DrawScale` or `FirstPersonModelScale`. Animated models take the same keys.
- The model is drawn, lit and seen by the camera like a game mesh. What it collides with is
  chosen by `collision`:
  - `"mesh"` (the default): its own triangles, as a game mesh does. Best for shapes that are not
    boxes; a pickup made of it can be picked up.
  - `"box"`: the box around its vertices. Cheaper, and closed even when the model has holes or
    open faces, but a shot or a player stops on the box, not on the shape.
  - `"none"`: nothing; players, characters and shots go through it (grass, light shafts…).

  Actors that the game gives a cylinder (`bUseCylinderCollision`, as most pickups) keep their
  cylinder for shots and traces.
- To stand a model in a map as a **solid decoration** (a wall, a crate, a car), add it with
  `model` instead of `class` (see [Placing things on maps](placing.md#placing-things-on-maps)): it blocks the
  player, the characters and the shots, as the map's own decorations do.
- The models are read once when the game starts; a missing or unreadable file gives one warning
  line and that model is left out.

Example: [examples/mods/medkit-crate](../../../examples/mods/medkit-crate/mod.toml).

## Animated models

A glTF model with a **skin** (a skeleton the vertices follow) and animations is read as an
animated model: a skeletal mesh of the game, like the characters and the weapons held in first
person. The same `[models.<Name>]` table is used; two more tables link it to the game:

```toml
[models.Guard]
file = "models/guard.glb"          # with a skin and its animations
scale = 50                         # optional, as for the other models

[models.Guard.animations]          # game animation = glTF animation
Fire = "shoot"
WalkF = { clip = "walk", notifies = [[0.25, "PlayFootStep"], [0.75, "PlayFootStep"]] }

[models.Guard.bones]               # game bone name = the model's bone (optional)
"X R Hand" = "hand_r"
```

- `Mod.<Name>` works wherever a skeletal mesh is accepted: `Mesh` of a character class or of an
  added actor, `MeshName` of a weapon (its view in first person, hands and gun).
- **The game's scripts play animations by name**: a soldier walking plays `WalkF`, a weapon
  being fired plays `Fire`, and so on. `[models.<Name>.animations]` makes each of these game
  animations from one of the model's glTF animations, which keep their own names too. The names
  each weapon and character uses are listed in [animations.md](../animations.md), with the
  notifies the game's animations call (footsteps, sounds of a reload…): give the same
  `notifies` (time from 0 to 1, script function) to have them. An animation that the model does
  not have is not played.
- `[models.<Name>.bones]` gives the game's bone names to the model's bones, for what the
  scripts attach to a bone by its name (a weapon in a character's hand, for example).
  [animations.md](../animations.md) lists the bones of the game's meshes.
- The animations are sampled at 30 frames per second. A sequence lasts as long as its glTF
  animation; one that loops should end on its first pose. glTF animations of translations,
  rotations and scales are read (linear, step or cubic spline); a bone's own scale is not kept.
- A part's base colour texture (a PNG next to the model) is drawn, as for the static models,
  or `material` draws a game texture.
- At most 254 bones, 4 bones per vertex (the strongest), 65535 vertices and 65535 triangles.
- The engine animates, draws and places the model as the game's own skeletal meshes: blends
  between animations, `AnimEnd`, notifies, bones turned by the scripts. For the shots, each bone
  gets a collision box around the vertices it moves most, as the game's meshes have: a shot
  knows which bone it hit, by the bone's own name: to be hit like a game character (the
  head…), name the model's bones as in [animations.md](../animations.md) (`[models.<Name>.bones]`
  only serves the scripts that ask for a bone by name, such as attachments).

The game also uses a weapon's first-person animation for its rate of fire: the next shot comes
when `Fire` ends. A mod's weapon fires as fast as its `Fire` animation is short, times its
`FireAnimRate` (see [Weapon animation speeds](weapons.md#weapon-animation-speeds)).
