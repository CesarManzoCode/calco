-- Corrutinas: generadores, paso de valores, wrap, estado, errores y productor-consumidor.
local function range(n)
  return coroutine.wrap(function() for i = 1, n do coroutine.yield(i) end end)
end
local acc = {}
for i in range(6) do acc[#acc + 1] = i end
print(table.concat(acc, " "))

local co = coroutine.create(function(a, b)
  print("start", a, b)
  local c = coroutine.yield(a + b)
  print("got", c)
  local d, e = coroutine.yield(c * 2)
  print("got", d, e)
  return "done", a + b + c + d + e
end)
print(coroutine.resume(co, 1, 2))
print(coroutine.status(co))
print(coroutine.resume(co, 10))
print(coroutine.resume(co, 3, 4))
print(coroutine.status(co), coroutine.resume(co))

local bad = coroutine.create(function() coroutine.yield(1) error("boom") end)
print(coroutine.resume(bad))
local ok, msg = coroutine.resume(bad)
print(ok, msg:match("boom"), coroutine.status(bad))

-- Productor, filtro y consumidor.
local function producer()
  return coroutine.create(function()
    for i = 1, 30 do coroutine.yield(i) end
  end)
end
local function filter(src, pred)
  return coroutine.create(function()
    while true do
      local _, v = coroutine.resume(src)
      if v == nil then return end
      if pred(v) then coroutine.yield(v) end
    end
  end)
end
local primes = filter(producer(), function(n)
  if n < 2 then return false end
  for d = 2, math.floor(math.sqrt(n)) do if n % d == 0 then return false end end
  return true
end)
local found = {}
while true do
  local _, v = coroutine.resume(primes)
  if v == nil then break end
  found[#found + 1] = v
end
print(table.concat(found, ","))

-- Permutaciones con corrutinas.
local function permgen(a, n)
  n = n or #a
  if n <= 1 then coroutine.yield(a) return end
  for i = 1, n do
    a[n], a[i] = a[i], a[n]
    permgen(a, n - 1)
    a[n], a[i] = a[i], a[n]
  end
end
local count, last = 0, nil
for p in coroutine.wrap(function() permgen({1, 2, 3, 4, 5}) end) do
  count = count + 1
  last = table.concat(p)
end
print(count, last)

print(coroutine.isyieldable(), coroutine.running() ~= nil)
local inner = coroutine.wrap(function() return coroutine.isyieldable() end)
print(inner())

-- Muchas corrutinas vivas a la vez.
local cos = {}
for i = 1, 200 do
  cos[i] = coroutine.create(function() local s = 0 for k = 1, i do s = s + coroutine.yield(k) end return s end)
  coroutine.resume(cos[i])
end
local total = 0
for round = 1, 200 do
  for i = round, 200 do
    local _, v = coroutine.resume(cos[i], round)
    if coroutine.status(cos[i]) == "dead" then total = total + v end
  end
end
print(total)
