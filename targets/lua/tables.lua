-- Tablas: secuencias, insert/remove/move/sort/concat/unpack y recorridos en orden fijo.
local t = {}
for i = 1, 20 do t[#t + 1] = (i * 37) % 23 end
print(table.concat(t, " "))
table.sort(t)
print(table.concat(t, " "))
table.sort(t, function(a, b) return a > b end)
print(table.concat(t, " "))

table.insert(t, 1, 100)
table.insert(t, 5, 200)
print(#t, table.remove(t), table.remove(t, 1), table.remove(t, 4), #t)
print(table.unpack(table.move({1, 2, 3, 4, 5}, 2, 4, 1)))
print(select("#", table.unpack({1, nil, 3}, 1, 3)), select(2, "a", "b", "c"))

local packed = table.pack(10, nil, 30)
print(packed.n, packed[1], packed[2], packed[3])

local records = {}
for i = 1, 12 do
  records[i] = {name = "item" .. string.format("%02d", (i * 5) % 12), score = (i * 7) % 10, id = i}
end
table.sort(records, function(a, b)
  if a.score ~= b.score then return a.score > b.score end
  return a.id < b.id
end)
for _, r in ipairs(records) do io.write(r.name, ":", r.score, " ") end
print()

-- Diccionario: se recorre en orden de claves ordenadas, nunca en el de pairs.
local dict = {}
for i = 1, 50 do dict["k" .. (i * 13) % 50] = i end
local keys = {}
for k in pairs(dict) do keys[#keys + 1] = k end
table.sort(keys)
local sum = 0
for _, k in ipairs(keys) do sum = sum + dict[k] end
print(#keys, keys[1], keys[#keys], sum)

-- Tabla mixta, longitud y next.
local mixed = {1, 2, 3, x = "x", [10] = 10}
print(#mixed >= 3, mixed.x, mixed[10], next({}) == nil)

-- Matriz y transpuesta.
local m = {}
for i = 1, 4 do
  m[i] = {}
  for j = 1, 5 do m[i][j] = i * 10 + j end
end
local tr = {}
for i = 1, 5 do
  tr[i] = {}
  for j = 1, 4 do tr[i][j] = m[j][i] end
end
for i = 1, 5 do print(table.concat(tr[i], " ")) end

-- Conjunto y operaciones.
local function set(list) local s = {} for _, v in ipairs(list) do s[v] = true end return s end
local a, b = set({1, 2, 3, 4, 5, 6}), set({4, 5, 6, 7, 8})
local inter, union = {}, {}
for i = 1, 10 do
  if a[i] and b[i] then inter[#inter + 1] = i end
  if a[i] or b[i] then union[#union + 1] = i end
end
print(table.concat(inter, ","), table.concat(union, ","))
