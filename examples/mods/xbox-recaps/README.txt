Xbox Loading Recaps
===================

What it does
------------
On the Xbox, while the next level loaded, a black screen showed comic bubbles with lines from
the story so far. The PC version ships the same texts (the [EndMsgMap] section of its language
files) but never shows them. This mod shows them again, in the language of your game:

- the bubbles appear as soon as the next level starts loading;
- once the level is ready, "Press to continue" appears at the bottom; a key, a mouse button, a
  gamepad button or a tap starts the level;
- a press before the level is ready skips to the usual loading screen, and the level then starts
  by itself.

It is not used for the co-op character screen, nor in a network co-op game.

How to install it
-----------------
Copy the "xbox-recaps" folder from Mods\_Examples into the Mods folder (next to _Examples), then
start the game. It is listed on the Mods page of the main menu, where it can be turned off and
on again. To remove it, delete its copy in Mods.

How to change it
----------------
Everything is in scripts\recaps.lua, a Lua script:

- CENTRES: where the bubbles go on the 640 x 480 screen, for one, two or three bubbles;
- OUTLINE: the colour of the bubbles' outlines ({red, green, blue}, 0 to 255);
- PROMPT: the "Press to continue" line in each language, and PROMPT_TOP its height;
- the on("loading", ...) handler chooses the text: replace text("EndMsgMap", key) with your own
  text to show something else, and remove hold_start() to never make the player wait.

The functions it uses (hud_text, hud_line, text_size, text, hold_start, release_start and the
events loading, loaded and loading_key) are explained in the modding guide, in
script\screen-and-loading.md.
