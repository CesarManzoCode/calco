-- Carga de rendimiento (contrato 8): cadenas, patrones, gsub, format y concatenación.
local words = {"alfa", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta",
               "iota", "kappa", "lambda", "mu"}
local parts = {}
local x = 7
for i = 1, 200000 do
  x = (x * 1103515245 + 12345) % 2147483648
  parts[i] = words[x % #words + 1] .. (x % 1000)
end
local text = table.concat(parts, " ")
print("len", #text)

local counts = {}
for w in text:gmatch("%a+") do counts[w] = (counts[w] or 0) + 1 end
local keys = {}
for k in pairs(counts) do keys[#keys + 1] = k end
table.sort(keys)
for _, k in ipairs(keys) do io.write(k, "=", counts[k], " ") end
print()

local digits = 0
for d in text:gmatch("%d+") do digits = digits + tonumber(d) end
print("digits", digits)

for round = 1, 5 do
  local up, n = text:gsub("(%a)(%a*)", function(a, b) return a:upper() .. b end)
  local swapped, m = up:gsub("(%u%l+)(%d+)", "%2%1")
  print("round", round, n, m, #swapped, swapped:sub(1, 40))
end

local lines = {}
for i = 1, 100000 do
  lines[i] = string.format("%06d|%-8s|%8.3f|%x", i, words[i % #words + 1], i / 7, i * 31)
end
local joined = table.concat(lines, "\n")
local sum = 0
for a, b, c in joined:gmatch("(%d+)|(%a+)%s*|%s*([%d%.]+)") do sum = sum + #b + tonumber(c) end
print(string.format("lines %d %.3f", #joined, sum))

local rev = {}
for i = 1, 20000 do rev[i] = string.reverse(parts[i]):rep(3, "-") end
print("rev", #table.concat(rev), rev[1], rev[20000])
print("find", text:find("kappa999", 1, true), select("#", text:byte(1, 100)))
