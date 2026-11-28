# New classes and new weapons

Classes a mod makes from the game's, new weapons, tracers, impacts and sparks. Back to [the modding guide](../README.md).

## New classes and new weapons

A mod can make new classes from the game's classes. A new class works exactly like the class it
is made from (same scripts, same behaviour), starts from its values, and gets its own values from
a class table, like any class:

```toml
[classes.Blaster]                  # a new class "Mod.Blaster"
parent = "XIII.Beretta"            # works like the 9mm pistol

[class."Mod.Blaster"]              # its own values
ItemName = "Blaster"
ReloadCount = 30
```

- `Mod.<Name>` works wherever a class is accepted: in class values (`AmmoName`,
  `AttachmentClass`, `InventoryType`…), in names the game loads by text (`PickupClassName`),
  for added actors (`[[map.<Map>.add]] class = "Mod.BlasterPick"`) and start weapons.
- Wherever the game asks whether an object is of the parent class, a new class answers yes: a
  "Mod.Blaster" is a Beretta for the game's scripts.
- Values set on the parent class by mods apply to the new class too, unless the new class sets
  its own.
- The name must not be one of the game's classes (the game would not know which one is meant);
  a wrong parent or a taken name gives one warning line and the class is not made.

**Add, not replace.** A new class adds something beside the game's own things; it never takes
the place of its parent. The parent only gives its behaviour and its starting values. To put
something in place of a game thing, a mod says so (`replace`, `[map.<Map>.replace]`, or values
set on the game class itself).

**A new weapon's slot and icon.** The game shows one weapon per slot (`InventoryGroup`) in its
weapon list, draws the slot's icon, and chooses weapons by slot (number keys, wheel). A new weapon
made from a game weapon therefore gets a slot of its own, beside its parent: the first free one
of 18, 19, 21, 22 and 23 (the slots no weapon of the game uses: slot 1 is the throwing
knives', the default of every weapon), with a log line
(`mods: Mod.Blaster: weapon slot 18 (InventoryGroup) of its own…`). `InventoryGroup = <n>` in its
class table chooses the slot itself. Its icon in the weapon list and the pickup message is its
`HudIcon` (a texture, `"Mod.<Name>"` for a PNG of the mod, 64 × 32 is the size the HUD draws),
else a plain grey pistol outline (`Mod.DefaultWeaponIcon`), never the parent's icon:

```toml
[textures.BlasterIcon]
file = "textures/blaster-icon.png"

[class."Mod.Blaster"]
HudIcon = "Mod.BlasterIcon"
```

A complete new weapon is four new classes: the weapon, the pickup that gives it, the gun seen
in a character's hand (attachment) and, if it has its own ammo, the ammo. Its own 3D model is
drawn for the pickup and the attachment; its own sounds and values go in its class tables:

| Class (parent) | Main values |
|---|---|
| Weapon (`XIII.Beretta`…) | `ItemName`, `AmmoName`, `PickupClassName`, `AttachmentClass`, `ReloadCount`, `PickupAmmoCount` (at most this many rounds in the magazine when picked up), `TraceAccuracy`, `hFireSound` |
| Pickup (`XIII.BerettaPick`…) | `InventoryType` (the weapon), `PickupMessage`, `StaticMesh` (the model on the ground) |
| Attachment (`XIII.BerettaAttach`…) | `WeaponClass` (the weapon), `StaticMeshName` (the model in hand), `hFireSound` |
| Ammo (`XIII.c9mmAmmo`…) | `ItemName`, `MaxAmmo`, `SRDamages`, `LRDamages` |

The weapon seen in first person (the hands and the gun) is the parent weapon's, unless the
weapon's `MeshName` names a mod's animated model (see [Animated models](textures-and-models.md#animated-models)):

```toml
[models.BlasterHands]              # hands and gun, with a skin and animations
file = "models/hands.glb"
scale = 200                        # the game draws first-person weapons at DrawScale 0.3

[models.BlasterHands.animations]   # the names the 9mm's scripts play (../animations.md)
Select = "draw"
Down = "holster"
Wait = "idle"
Fire = "shoot"
ReLoad = "reload"

[class."Mod.Blaster"]
MeshName = "Mod.BlasterHands"
```

The model's origin is the weapon's place in front of the eye (`PlayerViewOffset`), facing
where the player looks.

### Tracers, impacts and sparks

A weapon's shot effects are values of its ammo class, plus PNG textures of the mod. The
[blaster-gun](../../../examples/mods/blaster-gun/mod.toml) has a blue tracer at every shot, blue sparks
and a black burn mark where it hits; the other weapons keep their own effects.

**The tracer** is the streak drawn in first person from the middle of the screen to the gun, on
some shots. It is the ammo's `TBTexture`, stretched from the top-left corner of the image (the
middle of the screen) to its bottom-right corner (the gun), drawn translucent (black adds
nothing). `fTraceFrequency` is how many tracers per shot (9mm: 0.33, about one shot in three;
2.0 gives one at every shot). The game tints it orange; `TracerColor`, on the weapon class,
gives the colour instead (white draws the texture's own colours):

```toml
[textures.BlasterTracer]
file = "textures/blaster-tracer.png"   # a blue streak on black, corner to corner

[class."Mod.BlasterAmmo"]
TBTexture = "Mod.BlasterTracer"
fTraceFrequency = 2.0

[class."Mod.Blaster"]
TracerColor = { R = 255, G = 255, B = 255 }   # 0 to 255; A (opacity) is optional
```

**The impact** is an Emitter actor spawned where a shot hits (`ImpactEmitterMem` of the ammo),
and **the mark** left on walls a projector (`DecalProjectorClass`). The game picks both by the
surface hit (dust on concrete, chips on wood, sparks on metal…); `ImpactEffectsBySurface =
false` keeps the ammo's own on every surface (the surface's impact sound still plays). Shots
that hit a character still draw blood.

```toml
[class."Mod.BlasterAmmo"]
ImpactEmitterMem = "Mod.BlasterSparks"
DecalProjectorClass = "Mod.BlasterScorch"
ImpactEffectsBySurface = false
```

A new impact is made from one of the game's (`XIII.BulletMetalEmitter`, the ricochet on
metal: a flash, sparks and a puff of smoke; `XIII.BulletDustEmitter`, `XIII.WoodDustEmitter`…).
An Emitter is a list of particle emitters, `Emitters`; `"Emitters[n]" = { … }` changes the
values of the n-th one (from 0) for the new class only, the parent keeps its own. The values
are those of the game's `SpriteEmitter` (`Texture`, `MaxParticles`, `DrawStyle`,
`StartVelocityRange`, `ColorScale`, `LifetimeRange`…):

```toml
[textures.BlasterSpark]
file = "textures/blaster-spark.png"      # a blue glow on black

[classes.BlasterSparks]
parent = "XIII.BulletMetalEmitter"

[class."Mod.BlasterSparks"]
"Emitters[0]" = { Texture = "Mod.BlasterFlash" }                   # the flash
"Emitters[1]" = { Texture = "Mod.BlasterSpark", MaxParticles = 16 }  # the sparks
"Emitters[2]" = { DrawStyle = "PTDS_Darken" }                      # dark smoke
```

The mark is the game's bullet hole with another `ProjTexture`. A PNG is drawn *modulated* on
the wall: grey 128 leaves the wall as it is, darker darkens it, black makes it black. So a burn
mark is a dark shape on a plain grey 128 image:

The texture of a mark needs `clamp = true` in its `[textures.<Name>]` table, as the game's own
bullet holes: a mark is drawn over the whole triangle of the surface it hits, and a texture that
repeats would put copies of the mark all along a plank or a wall made of a few big triangles.

```toml
[textures.BlasterBurn]
file = "textures/blaster-burn.png"
clamp = true

[classes.BlasterScorch]
parent = "XIII.BulletScorch"

[class."Mod.BlasterScorch"]
ProjTexture = "Mod.BlasterBurn"
DrawScale = 0.35    # its size (9mm: 0.25)
Lifetime = 12       # seconds before it fades (9mm: 6)
```

Limits: the tracer is drawn only in the player's own first-person view, as in the game (not
for other characters' shots).
