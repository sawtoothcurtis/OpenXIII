# Scripts

A mod's `.toml` files say **what** changes: this character, this weapon, this object. A script
says **when** and **in what order**: a scene, a reaction, a small mission. Scripts are written
in Lua, a small language made for games; you can do a lot with the few words of these pages.

A mod's scripts are the `.lua` files of its `scripts` folder:

```
Mods/
  my-first-script/
    mod.toml
    scripts/
      hello.lua
```

They start each time a map starts (a checkpoint loads the map again, so they start again too),
in the order of their names. They work in the improved build, on the game that runs the level:
alone, in split screen, or on the host of a network game.

## Pages

1. [Your first script in 5 minutes](first-script.md)
2. [From toml to script](from-toml-to-script.md): the same things written both ways
3. [React to something](react.md): a player comes near, someone dies, a key is pressed
4. [Write a scene](scene.md): time, dialogue, orders, the camera
5. [Objectives, doors and the end of a level](objectives-doors-end.md)
6. [Invent a behaviour](behaviour.md): a character who acts on his own
7. [Build a mission](mission.md): a complete example
8. [A whole enemy in Lua: guard dogs](guard-dogs.md): the largest example
9. [A weapon with a script: the flamethrower](flamethrower.md): flames, characters on fire, marks on walls
10. [The screen and the loading screen](screen-and-loading.md): texts, counters, timers, bars, and a loading screen of your own
11. [All the functions](functions.md)

Three example mods of the repository use scripts: `examples/mods/guard-dogs` (a whole enemy
written in Lua: the dog the game's files hold but no level uses, with its animations and its
sounds, on several levels), `examples/mods/flamethrower` (a weapon whose flames, fires and marks
are a script) and `examples/mods/xbox-recaps` (the Xbox version's comic bubbles on the loading
screen between two maps). The other examples are toml only: what a mod.toml alone can do.
