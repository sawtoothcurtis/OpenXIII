# Troubleshooting

When a mod does not do what you expect, the game says why in one file: `mods-log.txt`.

## Recipe

1. Start the game, go where the mod should act, quit.
2. Open `Mods/mods-log.txt` in Notepad. It is written again at every start.
3. Look at the **Errors and warnings** part: each line names the mod, the file, the line
   number and what is wrong, with the closest valid names.

```text
warning: m/mod.toml: line 9: character "Jomes" is neither a look name (Characters tab of
Modding catalog.html) nor a [character.Jomes] of the mods (did you mean Jones?)
```

## Common causes

| What you see | Why, and what to do |
|---|---|
| The mod is not in the log | its folder has no `mod.toml`, or it is turned off on the Mods page |
| "file skipped" with a line number | a TOML mistake: a missing `"` or `]`, or a setting written twice |
| "unknown key …" | a typo in a setting's name: the message lists the settings allowed there |
| "unknown block …" | a typo in a block's name (`[[shortcutt]]`): the message gives the closest name |
| "did you mean …?" | a name is wrong: copy it from the catalog (see [find-names.md](find-names.md)) |
| "no actor named …" | the `who` of a `change` or the `by` of a `say` is not on that map: use `modwho` |
| "outside the map" | the position is in the void: take it again with `modpos` |
| A mod's loading screen stays and the level never starts | its script calls `hold_start()` and nothing calls `release_start()`: add `on("loading_key", function() release_start() end)` (see [script/screen-and-loading.md](script/screen-and-loading.md)), or turn the mod off |

Quotes must be straight quotes (`"`), the ones Notepad types. Names are not case-sensitive.
The game always starts: a wrong value is skipped and reported, the rest of the mod still
applies.

## Testing switches

| Switch | What it does |
|---|---|
| `--no-mods` | starts the game without any mod, to compare |
| Mods page (main menu) | turns one mod off for the next launch; conflicts show in red |
| F2, `modwho` / `modpos` | names what you look at, gives your position |

Two mods that set the same thing: the mod whose folder comes last (in name order) wins; the
log and the Mods page say which.

## On Android

Mods are written on a Windows computer. The Android build does not load mods yet: its
`improved` feature does not include mod support, and it has no Mods folder a phone's file
manager can reach. A mod made on a computer therefore plays on Windows only for now.

## If it still does not work

- Start from an example mod of `examples/mods` that works, and change one thing at a time.
- Check you run the improved build: the faithful build ignores the Mods folder.

All the details (log format, test variables, network identities):
[expert/loading-and-testing.md](expert/loading-and-testing.md).
