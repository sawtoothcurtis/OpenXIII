# Modding OpenXIII

**All the names you can use: [names/](names/README.md)** (characters, weapons, maps, sounds…).

A mod is a folder of text files (and, if you want, your own pictures, sounds and 3D models)
that changes the game: a guard who looks like Jones, a stronger pistol, a new character on the
beach, a line of dialogue, a keyboard shortcut that does something. Mods only work in the **improved build**
of OpenXIII; the faithful build has no mod code at all. Nothing of the original game is ever
modified: the game reads your files when it starts.

## The Mods folder

Next to the OpenXIII executable there is a folder named **Mods**. Each mod is one folder inside
it, with a file named `mod.toml`:

```
OpenXIII/
  Mods/
    jones-for-a-guard/
      mod.toml
```

Everything for mods is in the one `Mods` folder of the game:

```
Mods/
  your-mod/           a mod: active at the next launch
  _Examples/          the example mods that come with the game: none of them is active
  _Guide/             this guide, the names catalog and the lists of names
```

To use an example, copy its folder from `Mods/_Examples` into `Mods`, then start the game again.
The game never loads `_Examples` nor `_Guide`.

The **Mods** box of the main menu lists the mods of the Mods folder (each one is on until you
turn it off there) and turns them on or off; the change applies at the next launch. The mods
turned on come first, then those turned off. The game writes `Modding catalog.html` in
`Mods/_Guide` by itself when it starts, if it is missing or out of date: every name a mod can
use, with a search box.

## Your first mod in 5 minutes

1. In the Mods folder, make a folder `jones-for-a-guard` and in it a file `mod.toml` (Notepad
   is fine) with:

```toml
[mod]
name = "Jones for a guard"
version = "1.0.0"

[[map.PRock03.change]]
who = "gardien_lostpass"
look = "Jones"
```

2. Start the game, then **Load game > Select level > Doc Johansson** (the asylum, PRock03).
   The guard who loses his pass now looks like Major Jones.
3. Change `"Jones"` to `"Kim"`, `"Carrington"` or any name of the catalog's Characters tab,
   save, restart the game.

If nothing changes, open `Mods/mods-log.txt`: it says what was loaded and what was wrong.

**Words.** A mod file is made of blocks (`[mod]`, `[[map.Plage01.add]]`…), each with
settings (`name = "…"`, `position = [ … ]`). A key is only a key of the keyboard.

**Keep a mod tidy.** `mod.toml` holds the `[mod]` block (the mod's identity). Everything else
can go in other `.toml` files of the folder, read after `mod.toml` in name order: for example
`characters.toml`, `weapons.toml`, and one file per map in a `maps` sub-folder
(`maps/Plage01.toml`). The `scripts` sub-folder holds scripts: see [script/](script/README.md) (reactions to shots and hits, things kept on the screen, your own loading screen between two levels: [script/screen-and-loading.md](script/screen-and-loading.md)).

## I want to…

- [spawn a character](spawn-a-character.md) somewhere on a map, or with a keyboard shortcut
- [change a character](change-a-character.md) the map already places
- [change a weapon](change-a-weapon.md) (damage, magazine, rate of fire)
- [add a weapon](add-a-weapon.md) of my own
- [add an object or a decoration](add-an-object-or-decoration.md)
- [change sounds, ambience or music](sounds-ambience-music.md)
- [make a character talk](make-a-character-talk.md)
- [find the name of something](find-names.md)
- [make a keyboard shortcut](keyboard-shortcuts.md) (press K: a character appears…)
- [make something happen when…](reactions.md) XIII comes near someone, a character dies, a few seconds have passed
- [play my mod in multiplayer or co-op](multiplayer-and-coop.md)
- [fix a mod that does not work](troubleshooting.md)

Every detail of every feature is in the [expert pages](expert/loading-and-testing.md):
[characters](expert/characters.md), [weapons](expert/weapons.md),
[placing actors](expert/placing.md), [maps and checkpoints](expert/maps-and-checkpoints.md),
[dialogues](expert/dialogues.md), [sounds](expert/sounds.md),
[textures and models](expert/textures-and-models.md), [new classes](expert/new-classes.md),
[content and network games](expert/content-and-network.md).
