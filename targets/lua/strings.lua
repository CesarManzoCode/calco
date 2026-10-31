-- Cadenas y patrones: find, match, gmatch, gsub, rep, byte/char, reverse y format.
local text = [[
the quick brown fox jumps over the lazy dog
Pack my box with five dozen liquor jugs: 12 boxes, 345 jugs, 6789 dozens.
key1=value1; key2 = value two ;key3=  3.25
]]

local words = {}
for w in text:gmatch("%a+") do words[#words + 1] = w:lower() end
print("words", #words, words[1], words[#words])

local numbers = {}
for n in text:gmatch("%d+") do numbers[#numbers + 1] = tonumber(n) end
print("numbers", table.concat(numbers, ","))

local keys = {}
for k, v in text:gmatch("(%w+)%s*=%s*([^;\n]-)%s*;?\n?") do keys[#keys + 1] = k .. "->[" .. v .. "]" end
print("pairs", table.concat(keys, " "))

print(("hello world"):gsub("o", "0"))
print(("hello world"):gsub("(%w+) (%w+)", "%2 %1"))
print(("abc"):gsub("", "-"))
print(("%d items"):format(3), ("x"):rep(5, ","), ("Lua"):reverse())
print(string.find("a.b.c", ".", 1, true), string.find("a.b.c", "%.", 3))
print(string.match("2026-10-30", "(%d+)-(%d+)-(%d+)"))
print(string.match("  trim me  ", "^%s*(.-)%s*$") .. "|")
print(string.byte("ABC", 1, -1), string.char(72, 105))
print(("%q"):format("line\nwith \"quotes\"\0end"))
print(("[%5s][%-5s][%.2s]"):format("ab", "ab", "abcdef"))

local caesar = text:sub(1, 43):gsub("%a", function(c)
  local base = c:match("%l") and 97 or 65
  return string.char((c:byte() - base + 13) % 26 + base)
end)
print(caesar)

local counts = {}
for c in text:lower():gmatch("%a") do counts[c] = (counts[c] or 0) + 1 end
local letters = {}
for c in pairs(counts) do letters[#letters + 1] = c end
table.sort(letters, function(a, b)
  if counts[a] ~= counts[b] then return counts[a] > counts[b] end
  return a < b
end)
local top = {}
for i = 1, 8 do top[i] = letters[i] .. "=" .. counts[letters[i]] end
print("freq", table.concat(top, " "))

print(("balanced (a(b)c) here"):match("%b()"), ("THE (quick) fox"):find("%f[%a]%a+%f[%A]", 5))
local parts = {}
for field in ("a,b,,d"):gmatch("([^,]*)") do parts[#parts + 1] = "<" .. field .. ">" end
print(table.concat(parts))
