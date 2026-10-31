-- Recolector de basura: tablas débiles, finalizadores, modos y ciclos. No imprime cifras
-- de memoria, que dependen del asignador; solo hechos que el lenguaje garantiza.
collectgarbage("collect")
print(collectgarbage("isrunning"))

-- Tabla de valores débiles: lo que no tiene otra referencia desaparece.
local cache = setmetatable({}, {__mode = "v"})
local keep = {}
for i = 1, 100 do
  local obj = {id = i}
  cache[i] = obj
  if i % 10 == 0 then keep[#keep + 1] = obj end
end
collectgarbage("collect")
local alive = 0
for i = 1, 100 do if cache[i] then alive = alive + 1 end end
print("weak values alive", alive)

-- Tabla de claves débiles (efemérides).
local eph = setmetatable({}, {__mode = "k"})
local k1, k2 = {}, {}
eph[k1] = {k1}
eph[k2] = "two"
eph[{}] = "lost"
k2 = nil
collectgarbage("collect")
local n = 0
for _ in pairs(eph) do n = n + 1 end
print("weak keys alive", n, eph[k1][1] == k1)

-- Finalizadores: corren al recolectar, en orden inverso de marcado.
local order = {}
for i = 1, 5 do
  setmetatable({}, {__gc = function() order[#order + 1] = i end})
end
collectgarbage("collect")
print("finalized", table.concat(order, ","))

-- Resurrección: el finalizador guarda el objeto y luego sigue siendo usable.
local saved
do
  local o = setmetatable({name = "phoenix"}, {__gc = function(self) saved = self end})
  o = nil
end
collectgarbage("collect")
print("resurrected", saved and saved.name)

-- Ciclos grandes sin referencias externas.
for round = 1, 20 do
  local head = {}
  local cur = head
  for i = 1, 2000 do cur.next = {prev = cur, i = i} cur = cur.next end
  cur.next = head
end
collectgarbage("collect")
print("cycles collected")

-- Modos incremental y generacional, y step.
print(collectgarbage("incremental"))
print(collectgarbage("generational"))
local t = {}
for i = 1, 50000 do t[i] = {i} end
t = nil
collectgarbage("step", 0)
print(collectgarbage("incremental"))
collectgarbage("stop")
print(collectgarbage("isrunning"))
collectgarbage("restart")
print(collectgarbage("isrunning"))

-- Muchas cadenas temporales.
local s = 0
for i = 1, 30000 do s = s + #(tostring(i) .. "-" .. i * 2) end
collectgarbage("collect")
print("strings", s)
print(type(collectgarbage("count")))
