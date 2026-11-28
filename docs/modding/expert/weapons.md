# Weapons in detail

Every weapon setting: class values, animation speeds, fire modes, how a weapon is held, another weapon's look in first person. Back to [the modding guide](../README.md).

## Changing weapons

A mod can change the default value of any property of any class of the game, starting with
weapons and ammo, with `[class."Package.Class"]` tables. They can go in `mod.toml` or in other
`.toml` files of the mod folder (read after `mod.toml`, in name order).

```toml
[class."XIII.Beretta"]          # the 9mm pistol
ReloadCount = 17                # magazine size (original: 13)
TraceAccuracy = 0.8             # original: 1.5
FireOffset = { Y = 4.0 }        # struct: only the given fields change

[class."XIII.c9mmAmmo"]         # its ammo
SRDamages = 35                  # short range damage (original: 25)
MaxAmmo = 300                   # most 9mm ammo XIII can carry (original: 200)
```

`weapons.md` lists every weapon and ammo class with its properties, their types and
the original values. It is generated, with `characters.md`, from the game's own
scripts by `oxiii mods-reference <game folder> docs/modding` (oxiii built with `--features mods`).
A complete example is in [examples/mods/stronger-pistol](../../../examples/mods/stronger-pistol/mod.toml).

Values:

| Property type | Write | Example |
|---|---|---|
| int, byte | a whole number | `ReloadCount = 17` |
| float | a number | `ShotTime = 0.35` |
| bool | `true` or `false` | `bUseSilencer = false` |
| enum (byte) | the value's name | `WeaponMode = "WM_Auto"` |
| string, name | a string | `ItemName = "Big 9mm"` |
| struct (vector…) | an inline table; only the given fields change | `FireOffset = { X = 9.0, Z = -2.0 }` |
| one element of a static array | the key `"Name[index]"` | `"Sounds[2]" = …` |
| dynamic array | an array (replaces the whole array) | `List = [1, 2, 3]` |
| object or class reference | a string | `AmmoName = "XIII.c9mmAmmo"` (see the limits) |
| values of a sub-object (an Emitter's particle emitters) | an inline table; the class gets its own copy | `"Emitters[1]" = { MaxParticles = 16 }` (see [Tracers, impacts and sparks](new-classes.md#tracers-impacts-and-sparks)) |

Rules:

- The values replace the original ones when the game loads the class, after the game's .ini and
  language files. Subclasses inherit them unless they set their own value: `XIII.BerettaPlus`
  inherits the Beretta's `ReloadCount`, and `XIII.c9mmAmmoPeace` keeps its own `MaxAmmo` (999)
  but inherits `SRDamages` from `XIII.c9mmAmmo`.
- Properties work together as in the original scripts. For example a new weapon's magazine is
  filled with at most `PickupAmmoCount` rounds, so a bigger `ReloadCount` alone only shows
  after the first reload (the example mod raises both).
- When two mods set the same property, the mod whose folder comes last wins; the log says so.
- An unknown class or property, or a value of the wrong type, gives one warning line in the log
  (mod, file and line, class, property) and the value is ignored. The game always starts.
- The log lists every value applied, for example
  `mods: XIII.Beretta.ReloadCount = 17 (mod stronger-pistol, …\mod.toml line 11)`.

Limits of this first version:

- Sounds, textures and meshes are found in any package of the game (see
  [Content from any level](content-and-network.md#content-from-any-level)); a script object (an actor, for example)
  must exist when the class is loaded, otherwise the value is ignored with a warning.
- These are *default* values: they apply to every weapon created afterwards. A weapon that the
  player carries from one level to the next keeps travel values such as its ammo count.
- The meaning of a property comes from the original scripts. Some are only used by the
  enemies: for example `ShotTime` is, per its comment in the game's sources, used by the AI
  controllers. The player's rate of fire is not a property: the next shot comes when the
  weapon's firing animation ends (see [Weapon animation speeds](#weapon-animation-speeds)).

### Weapon animation speeds

The game waits for a weapon's first-person animations: the next shot comes when the firing
animation ends, the magazine is full when the reload animation ends, the weapon can be used when
its select animation ends. The scripts play them at speed 1.0. Three settings of the mods, written
in the same class tables (global or `[map.<Map>.class."..."]`), multiply these speeds:

```toml
[class."XIII.Beretta"]
FireAnimRate = 2.0      # fires twice as fast (rate of fire)
ReloadAnimRate = 1.5    # reloads 1.5 times as fast
SelectAnimRate = 0.5    # draws and puts the gun away twice as slowly
```

| Setting | Animations it speeds up | 1.0 |
|---|---|---|
| `FireAnimRate` | firing (`PlayFiring`, `PlayAltFiring`, the fists' second attack, a grenade being armed) | the original game |
| `ReloadAnimRate` | reload (`PlayReloading`; with two pistols, the one put down while the other reloads) | the original game |
| `SelectAnimRate` | drawing and putting away (`PlaySelect`, `TweenDown`) | the original game |

- A value is a number above 0 and at most 20. Subclasses and new classes made from the class
  get it too, unless they set their own; a map table wins on its map.
- They are not properties of the game's classes: the log lists them like the other values
  (`mods: XIII.Beretta.FireAnimRate = 2.0 (mod …)`), but `weapons.md` does not.
- They change the weapon's own animation, so the sound and the other events of the animation
  come sooner too. Enemies holding the weapon also use it; their pace between shots is mostly
  `ShotTime`.
- On the PC used for the tests, the 9mm fires again after 0.30 s, reloads in 1.87 s and is put
  away in 0.67 s; with `FireAnimRate = 2.0`, `ReloadAnimRate = 3` and `SelectAnimRate = 0.5`:
  0.15 s, 0.63 s and 1.33 s.

### Fire modes and how a weapon is held

These are properties of the game's weapon classes (`weapons.md` gives every weapon's value):

| Property | Class | Values | What it changes |
|---|---|---|---|
| `WeaponMode` | weapon | `"WM_Auto"` (fires while the button is held), `"WM_SemiAuto"` (one shot per press), `"WM_Burst"` (three shots per press) | the player's fire mode; the pace is still the firing animation |
| `FiringMode` | weapon | see below | how characters hold the weapon in third person (standing, walking) |
| `FiringMode`, `AltFiringMode` | attachment (`XIII.BerettaAttach`…) | see below | the third-person firing, reload and weapon change animations (the weapon's `FiringMode` by default, see below) |
| `WHand` | weapon | `"WHA_1HShot"` (one hand), `"WHA_2HShot"` (two hands), `"WHA_Throw"`, `"WHA_Fist"`, `"WHA_Deco"` | with something in the left hand, a two-handed weapon cannot be chosen |

`FiringMode` names, with the weapons of the game that use them: `FM_1H` (9mm, magnum, mini Uzi),
`FM_2H` (hunting rifle, pump shotgun, crossbows, harpoon), `FM_M16` (M16, Kalashnikov),
`FM_2HHeavy` (M60), `FM_Bazook` (bazooka), `FM_Snipe` (sniper rifle), `FM_Throw` (grenades,
knives, darts), `FM_Fists`, `FM_Stun` (chairs, bottles…); alternate fire: `FM_BazookAlt`,
`FM_ARAlt`, `FM_ShotGunAlt`, `FM_HarpoonAlt`, `FM_44Alt`. Any other name plays the pistol's
animations. The weapon's value is enough: a mods build also gives it to the weapon's attachment
class (its `AttachmentClass`), unless a mod sets the attachment's `FiringMode` itself, and the
log says so:

```toml
[class."Mod.Blaster"]
FiringMode = "FM_M16"     # held and fired like an assault rifle in third person
```

`mods: Mod.Blaster.FiringMode = "FM_M16" also on its attachment Mod.BlasterAttach, for the
third-person animations (…)`. This follows class values; a `[map.<Map>.class."..."]` value only
changes the actors placed in that map.

On the PC used for the tests, the fire button held for 1.5 s fires the 9mm once in
`WM_SemiAuto` (the game's value), 6 times in `WM_Auto` and 3 times in `WM_Burst`.

**Two fire buttons.** What the second fire button does comes from the weapon's scripts, so a new
weapon gets it from its parent class. The [blaster-gun](../../../examples/mods/blaster-gun/mod.toml) is
made from the .44 (`XIII.magnum`): the fire button shoots once per press, and the second fire
button, held, shoots again and again. It keeps the 9mm's hands with `FirstPersonFrom`, plays the
9mm's firing animation for the .44's second fire (`FirstPersonAnimations = { FireAlt = "Fire" }`)
and shoots faster with `FireAnimRate`: about 8 shots per second while the button is held.

```toml
[classes.Blaster]
parent = "XIII.magnum"             # one shot per press; second fire button held: a volley

[class."Mod.Blaster"]
AltAmmoName = "Mod.BlasterAmmo"    # the .44's second fire uses AltAmmoName
FirstPersonFrom = "XIII.Beretta"
FirstPersonAnimations = { FireAlt = "Fire" }
FireAnimRate = 2.2
ViewFeedBack = { X = 1.5, Y = 3.0 }   # the 9mm's kick: the .44's lifts the view in a volley
```

### Another weapon's look in first person

A weapon is seen in first person with the hands and gun of its own mesh (`MeshName`), and its
scripts play their own animation names on it. `FirstPersonFrom` gives it the first-person look
of another weapon while it keeps its own behaviour (its scripts, ammo, magazine…):

```toml
[classes.Rifle9]
parent = "XIII.Beretta"            # behaves as the 9mm pistol

[class."Mod.Rifle9"]
FirstPersonFrom = "XIII.m16"       # seen in first person as the M16
FirstPersonAnimations = { FireLast = "Fire" }   # optional
```

- `FirstPersonFrom` copies the other weapon's `MeshName`, `PlayerViewOffset`, `DrawScale`,
  `FireOffset`, `AltFireOffset` and muzzle flash values (`FirstPersonMFClass`, `FPMFRelativeLoc`,
  `FPMFRelativeRot`, `bDrawMuzzleFlash`, `MuzzleScale`, `MuzzleFlashSize`, `MFTexture`,
  `FlashOffsetX`, `FlashOffsetY`, `FlashLength`) into the class defaults. Values the mod sets
  itself in the same table win (`PlayerViewOffset = { Z = -7.0 }` changes only Z).
- The scripts still play their own names (the 9mm plays `Fire`, `FireLast`, `FireVide`,
  `Select`, `Down`, `ReLoad`, `Wait`…). A name the new mesh does not have is played as the closest
  name it has: without the endings `Vide`, `Last`, `Act` (`WaitActVide`, then `WaitAct`, then
  `Wait`), without a variant number (`Fire2` as `Fire`), or with the first variant (`Fire` as
  `Fire1`). `FirstPersonAnimations` gives names directly: script name = name of the mesh's
  animation. `animations.md` lists the animations of each weapon's mesh. A name with no match is
  played as is, which plays nothing: the log says so once, with the mesh's animations.
- Since the next shot comes when the firing animation ends, the borrowed animations also give
  the rate of fire (the M16's `Fire` is short: the 9mm made to look like it fires again after
  0.07 s instead of 0.30 s); `FireAnimRate` changes it.
- These two are class defaults: they go in `[class."..."]` tables, not in map tables. The class
  must not be the other weapon nor one of its parents.

**Other arms.** `[arms.<Name>]` remakes first-person meshes of the player's game when it starts: `from` is a mesh or `"Package.*"` (every mesh of the package with first-person arm bones: `["XIIIArmes.*", "XIIIDeco.*"]` are all the weapons and objects held), `replace = true` draws them instead of the game's (otherwise they are `Mod.<Name>.<Mesh>`, or `Mod.<Name>` for a single mesh, to give as a weapon's `MeshName`), `skip` leaves meshes out. `fingers`, `hand`, `wrist`, `forearm` make them thinner (1 = unchanged, 0.3 to 1.5), also per mesh in `[arms.<Name>.mesh."Package.Mesh"]` (where `cuff` also moves the sleeve's cuff on that mesh); `skin = [r, g, b]` or `skin_from` (a character model whose hands and forearms are measured) recolours the skin and `nails` the nails. A character model to measure can be given with another of its skins: `"XIIIPersos.SheridanM@XIIIPersos.Skins.SheridanChicTex"`. The weapons, objects and animations stay the game's. A sleeve over the forearm (a uniform, a jacket) is an `[arms.<Name>.sleeve]` table: `fabric = [r, g, b]` or `fabric_from` (a character model whose arms are measured), `cuff` (where it ends, 0 at the elbow to 1 at the wrist) an optional `band` colour and `band_width` (at its edge, or from `band_at` along the forearm), `creases = true` for two thin ink lines, `rings` (thin ink lines all around, along the forearm; in `ring_colour` and `ring_width` instead for braid stripes) a strap `tab = [from, to, angle, half width]` and a ribbed knit cuff `ribs = [from, how many around, width of each]` with `rib_colour = [r, g, b]`, a plaid `checks = [spacing along, lines around, width of each]` with `check_colour`, and `buttons = [first along the forearm, how many, angle, spacing]` with `button_colour`; `relief_from` (a character model) adds a sleeve in relief: a shell built around each forearm (`thickness`, `room`), with a lip at the cuff (`lip`), narrowing towards the cuff with `taper = [from, factor at the cuff]` and soft folds (`folds`, `fold_depth`, `fold_width`, `fold_slant`), drawn plain in the fabric's colour (with its band and creases, in a texture of its own) so that the game's cartoon shading gives its shadows (`relief_texture = "model"` uses that model's own printed cloth instead, a camouflage: the lit half of its sleeve, mirrored all around). Gloves are an `[arms.<Name>.gloves]` table: `colour = [r, g, b]` or `colour_from` (a character model whose hands are measured), from `from` along the forearm (1 = the wrist), up to `to` for fingerless gloves (the hand is at 1.5, its fingers at 2: `to = 1.75`). `hide_arm = "left"` (or `"right"`) leaves that arm out of every mesh, for a one-armed character: what it held stays where the game's animation puts it. Use one arms mod at a time.

**A model of the mod in the hand.** Without an animated model of hands and gun, a new weapon can
keep its parent's hands and animations, hide the parent's gun and hold a static model instead,
the way the game holds the 9mm's silencer (an attachment on a bone of the hands mesh):

```toml
[class."Mod.Blaster"]
FirstPersonModel = "Mod.BlasterGun"     # a static mesh: a mod's model or a game mesh
FirstPersonHideBones = ["Berretta", "chargeur", "percuteur"]   # the 9mm's gun, shrunk to nothing
FirstPersonModelScale = 0.3             # optional, 1.0 by default
FirstPersonModelBone = "Berretta"       # optional: the bone it is attached to (default: the
                                        # weapon's own gun bone, GetBaseWeaponBone)
FirstPersonModelOffset = { X = 0, Y = 0, Z = 0 }              # optional: moves it
FirstPersonModelRotation = { Pitch = 0, Yaw = 0, Roll = 0 }   # optional: turns it
```

The bone names of each first-person mesh are in `animations.md` (the 9mm's gun is `Berretta`
with `chargeur` and `percuteur`). The model follows the hands' animations (firing, reloading).
These are class defaults, like `FirstPersonFrom`.

**Its place in the hand.** Without any setting, the model's origin is on the bone and its front
points where the player looks: a model made facing forward in a 3D editor (glTF -Z, the game's
+X) is held facing forward. The two settings then move and turn it, as seen by the player:

- `FirstPersonModelOffset`, in game units: `X` forward, `Y` to the right, `Z` up. The weapon
  is close to the eye, so small values go far (1 to 5).
- `FirstPersonModelRotation`, in game angle units (65536 = a full turn, 16384 = 90°, 8192 =
  45°): `Pitch` raises the front, `Yaw` turns it to the right, `Roll` tilts it sideways.

```toml
[class."Mod.Blaster"]
FirstPersonModelOffset = { X = 2.0, Z = -0.5 }   # 2 units further forward, half a unit lower
FirstPersonModelRotation = { Yaw = -1024 }       # its front 5.6° to the left
```

**Its muzzle.** The front end of the model (its farthest point forward, in the middle of the
points around it) is the muzzle: the tracer the weapon draws in first person (the ammo's
`TBTexture`) ends there, as the view draws the model, whatever the screen's shape (wide screens,
split screen); `FlashOffsetX` and `FlashOffsetY` are then not used. A script reads that point
as the weapon's `muzzle` field (`p:get("Weapon").muzzle`) to put an effect on it (the
flamethrower's flame).

**Seen in a character's hand.** The attachment class (`AttachmentClass`) shows its
`StaticMeshName`, the same model or another. Its `MuzzleOffset` is where the muzzle flash and
the bullet trail start, from the hand's bone: the gun's place in the hand (the weapon's
`ThirdPersonRelativeLocation`) plus the tip of the model, at its `DrawScale`.
