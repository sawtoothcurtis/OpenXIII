-- Three comic-book insets, one after the other, each with its own camera (panel in the scripts'
-- reference). They show on every player's screen; the game goes on behind them.
if map() ~= "Plage01" then return end

on("event", "Objectif91", function(player)   -- the opening scene is over
  wait(1)
  local at = player.position
  panel{
    -- From the side, with a caption; the next inset comes one second later.
    {from = {at[1] + 150, at[2] + 60, at[3] + 60}, look_at = player, see_player = true,
     seconds = 4, x = 40, y = 40, text = "XIII?", appear = "left", sound = "tip", after = 1},
    -- From above, with one of the levels' onomatopoeia pictures.
    {from = {at[1] - 150, at[2] - 60, at[3] + 120}, look_at = player, see_player = true,
     seconds = 3, x = 200, y = 40, word = "plage01a", appear = "zoom", after = 1},
    -- From the door, looking at the place where he stands.
    {from = {at[1], at[2] + 250, at[3] + 40}, look_at = {at[1], at[2], at[3]}, see_player = true,
     seconds = 2, x = 360, y = 40, appear = "above"},
  }
end)
