# Your first script in 5 minutes

Show a message on the beach, then another one ten seconds later.

## Recipe

1. In the Mods folder, make a folder `my-first-script` with a `mod.toml`:

```toml
[mod]
name = "My first script"
version = "1.0.0"
```

2. In it, make a folder `scripts` and in it a file `hello.lua`:

```lua
if map() ~= "Plage01" then return end   -- only on the beach

on("start", function(player)
  message("Hello from my first script.")
  wait(10)                               -- ten seconds of game time
  message("Ten seconds later.")
end)
```

3. Start the game and play the beach (**Load game > Select level > Brighton Beach**).

## Useful to know

- `on(event, ..., function ... end)`: what to do when something happens. The code outside the
  functions runs once, when the map starts.
- `wait(seconds)` pauses only this function; the game goes on.
- `spawn{character = "Guard", near = player}` creates a character next to the player (the
  first spot around him with room and ground).
- `log("text")` writes a line in `Mods/mods-log.txt`: handy to see what your script does.
- After changing a script, open the console (F2) and type `modreload`: the scripts of the map
  start again from their files, without leaving the game.

## If it does not work

- Open `Mods/mods-log.txt`: an error in a script is written there with the file and the line
  (`my-first-script/scripts/hello.lua:4: ...`). The game never stops because of a script.
- Nothing at all: check that the file ends in `.lua` and is in the `scripts` folder.
- A script that never stops (a loop without `wait`) is stopped after a moment, with one line
  in the log.
