-- A bodyguard: a character who follows the player and protects him.
--
-- behaviour(name, fn) writes how a character acts: fn(self) runs like a handler (wait,
-- wait_until, orders) for as long as the character lives. spawn{..., behaviour = name} or
-- someone:behave(name) gives it to a character.

if map() ~= "Plage01" then return end

behaviour("Bodyguard", function(self)
  local boss = player()
  while self.alive and boss.alive do
    -- Someone fighting the boss? (a character whose AI has him as its enemy)
    local threat
    for _, someone in ipairs(characters_near(self, 1500)) do
      local ai = someone:get("Controller")
      if someone ~= self and ai and ai:get("Enemy") == boss then
        threat = someone
        break
      end
    end
    if threat then
      message("Your bodyguard fights " .. threat.name .. ".")
      self:attack(threat)
      wait_until(function() return not threat.alive end, 15)
    else
      self:follow(boss)   -- walks back to him when he is more than a few steps away
      wait(1)
    end
  end
end)

-- After the opening scene, he comes a few steps from the player.
on("event", "Objectif91", function(player)
  local at = player.position
  for i = 1, 8 do
    local spot = {at[1] + 250 * math.cos(i), at[2] + 250 * math.sin(i), at[3]}
    if spawn{character = "Bodyguard", position = spot, behaviour = "Bodyguard"} then
      message("A bodyguard has joined you.")
      return
    end
  end
  log("no room for the bodyguard")
end)
