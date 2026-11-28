-- The Xbox version's loading recaps: comic bubbles on a black screen while the next level loads.
--
-- The texts come from the game's language files: the [EndMsgMap] section of Engine.<language>
-- has one entry per map being loaded, with lines from the level before it. "@" separates the
-- bubbles (three at most), "|" the lines inside a bubble. After a lost mission the entry is
-- "mapmenu".
--
-- Everything is drawn on a 640 x 480 screen (the console's), which the game scales to the window.

-- Where the bubbles go, by how many there are: the middle of each one.
local CENTRES = {
  {{320, 240}},
  {{100, 200}, {540, 280}},
  {{100, 100}, {540, 230}, {100, 360}},
}
local LINE_HEIGHT = 22
local OUTLINE = {196, 196, 196}
-- "Press to continue", once the level is ready, by language of the game's texts.
local PROMPT = {
  int = "Press to continue",
  frt = "Appuyez pour continuer",
  det = "Zum Fortfahren drücken",
  itt = "Premi per continuare",
  est = "Pulsa para continuar",
}
local PROMPT_TOP = 448

-- A point of the 640 x 480 screen as the hud_ functions take it (0 to 1).
local function at(x, y) return {x / 640, y / 480} end

-- The width of a line of text, in pixels of the 640 x 480 screen.
local function width(line)
  local w = text_size(line)
  return math.floor(w * 640 + 0.5)
end

-- Splits text on a separator: every piece, empty ones included.
local function split(text, sep)
  local out, from = {}, 1
  while true do
    local i = string.find(text, sep, from, true)
    if not i then out[#out + 1] = string.sub(text, from) return out end
    out[#out + 1] = string.sub(text, from, i - 1)
    from = i + 1
  end
end

-- A bubble's top left corner: centred on (cx, cy), kept 50 pixels inside the screen's edges.
local function place(cx, cy, w, h)
  local x, y = cx - w // 2, cy - h // 2
  if x < 50 then x = 50 end
  if x >= 570 - w then x = 570 - w end
  if y < 50 then y = 50 end
  if y >= 430 - h then y = 430 - h end
  return x, y
end

-- Bubble number i (1 to 3) of text size w x h at (x, y): its outline (with its tail) and its lines.
local function bubble(i, lines, x, y, w, h)
  local r, b = x + w + 40, y + h + 20
  local tail = (i == 1 and 50) or (i == 2 and w - 60) or 100
  local a, c = x + tail, x + tail + 18
  local any = false
  for _, l in ipairs(lines) do if l ~= "" then any = true end end
  if any then
    local segments = {{x, y, r, y}, {x, y, x, b}, {r, y, r, b}, {x, b, a, b}, {c, b, r, b}}
    -- The second bubble's tail leans the other way.
    if i == 2 then
      segments[6], segments[7] = {a, b, a, b + 30}, {a, b + 30, c, b}
    else
      segments[6], segments[7] = {a, b, c, b + 30}, {c, b + 30, c, b}
    end
    for _, s in ipairs(segments) do
      hud_line{from = at(s[1], s[2]), to = at(s[3], s[4]), colour = OUTLINE}
    end
  end
  for k, l in ipairs(lines) do
    hud_text{text = l, x = (x + 20) / 640, y = (y + 8 + LINE_HEIGHT * (k - 1)) / 480, colour = "white"}
  end
end

on("loading", function(from, to, failed)
  local key = failed and "mapmenu" or to
  local recap = text("EndMsgMap", key)
  if recap == nil then return end
  local bubbles = split(recap, "@")
  local n = math.min(#bubbles, 3)
  for i = 1, n do
    local lines = split(bubbles[i], "|")
    local w = 0
    for _, l in ipairs(lines) do w = math.max(w, width(l)) end
    local h = LINE_HEIGHT * #lines
    local x, y = place(CENTRES[n][i][1], CENTRES[n][i][2], w, h)
    bubble(i, lines, x, y, w, h)
  end
  -- The next level waits for a press.
  hold_start()
end)

-- Once the level is ready: "Press to continue", centred at the bottom.
on("loaded", function()
  local line = PROMPT[game().language] or PROMPT.int
  hud_text{text = line, x = ((640 - width(line)) // 2) / 640, y = PROMPT_TOP / 480, colour = "white"}
end)

-- A press: the level starts (at once if it is ready, otherwise behind the usual loading screen).
on("loading_key", function()
  release_start()
end)
