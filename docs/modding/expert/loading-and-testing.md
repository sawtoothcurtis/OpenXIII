# Builds, loading and testing mods

How the game finds and loads mods, the mod.toml identity table, the mods log and the testing switches. Back to [the modding guide](../README.md).

## Which builds have mod support

OpenXIII is built in two versions:

- **The faithful build** (no cargo feature) is the faithful reproduction of the original game.
  It has **no mod support**: the mod code is not even compiled in.
- **The improved build** (feature `improved`) adds fixes that the original game does not have,
  **and includes mod support** (it turns on the `mods` feature). In the improved build,
  `--no-mods` (or `OXIII_NO_MODS=1`) gives the improved game without mods.

```bash
cargo build --release -p openxiii-viewer --features improved
```

The `mods` feature can also be turned on alone (`--features mods`), for working on mods on top
of the faithful game.

## The Mods folder

Each mod is a sub-folder of the **Mods** folder. By default the Mods folder is the folder named
`Mods` next to the OpenXIII executable. The `OXIII_MODS` environment variable can point to
another folder.

```
OpenXIII/
  oxiii-view.exe
  Mods/
    example/
      mod.toml
    another-mod/
      mod.toml
```

Mods are loaded in the order of their folder names (plain byte order: upper case before lower
case, so `Zeta` comes before `alpha`).

### Turning a mod off

The **Mods** box of the main menu lists the mods of the folder (name, version, on or off). A
mod turned off there is written to `disabled-mods.txt` in the Mods folder, one folder name per
line (`#` starts a comment; you can also edit this file by hand). The mods listed in it are not
loaded. Mods are loaded when the game starts: a change applies at the next launch.

## mod.toml

Every mod folder must contain a `mod.toml` file with a `[mod]` table:

```toml
[mod]
name = "Example mod"           # required
version = "1.0.0"              # required
author = "Your name"           # optional
description = "What it does"   # optional
openxiii = "0.1.0"             # optional: the OpenXIII version the mod expects
```

- A malformed `mod.toml` (bad syntax, missing `name` or `version`, unknown key) is reported in the
  log with the file, the line and what is wrong. That mod is skipped; the game still starts.
- A sub-folder without `mod.toml` is ignored, with a warning in the log.
- A missing or empty Mods folder simply means no mods.

**Files of a mod.** After `mod.toml` the game reads the other `.toml` files of the mod folder in
name order, then the `.toml` files of its `maps` sub-folder in name order (by convention one
file per map: `maps/Plage01.toml`). Any table can go in any of these files. The files a mod
names (`file = "sounds/shot.wav"`, models, textures) are always found from the mod folder,
also from a file of `maps`. The `scripts` sub-folder is reserved and never read as tables.

At startup the log shows one line per mod and a total, for example:

```
mods: loaded "Example mod" version 1.0.0 (folder <OpenXIII folder>\Mods\example)
mods: 1 mod(s) loaded from <OpenXIII folder>\Mods
```

### mods-log.txt

When the Mods folder exists, the game also writes **`Mods/mods-log.txt`** at every start (the
previous one is replaced; without a Mods folder nothing is written). It is plain text, easy to
read without the console:

- each mod folder found, `[loaded]` or `[NOT LOADED]` with the reason (the file, the line and
  what is wrong);
- for each loaded mod, a short list of what it adds or replaces (class values, new classes, map
  changes, characters, keys, sounds, textures, models, sound banks and packages);
- **Errors and warnings**: every problem met so far, with the mod, the file and the line
  (unknown class or property, wrong value, unknown actor name, conflict between two mods…);
- every `mods:` line of the run, in order, including those written when a level starts or a
  mod key is pressed: the file is rewritten as they come, so it is complete even if the game
  is closed or crashes.

```
[loaded] stronger-pistol: "Stronger Pistol" version 1.0.0
    changes 4 class value(s): XIII.Beretta (3), XIII.c9mmAmmo (1)
    gives XIII.Beretta at the start of Plage01

Errors and warnings: 1
    warning: XIII.Beretta.ReloadCont: XIII.Beretta has no property ReloadCont (mod stronger-pistol, …\mod.toml line 11), value ignored
```


## Turning mods off

Any of these starts the game without mods:

- the `--no-mods` command-line option;
- the environment variable `OXIII_NO_MODS=1`;
- an empty (or missing) Mods folder.
