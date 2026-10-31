-- Carga de rendimiento (contrato 8): árboles binarios, cierres, metatablas, corrutinas y
-- ordenación. Azar con semilla fija.
math.randomseed(42)

local function bottom_up(depth)
  if depth == 0 then return {} end
  depth = depth - 1
  return {bottom_up(depth), bottom_up(depth)}
end

local function check(tree)
  if tree[1] then return 1 + check(tree[1]) + check(tree[2]) end
  return 1
end

local maxdepth = 15
print("stretch", check(bottom_up(maxdepth + 1)))
local long_lived = bottom_up(maxdepth)
for depth = 4, maxdepth, 2 do
  local iters = 1 << (maxdepth - depth + 4)
  local c = 0
  for _ = 1, iters do c = c + check(bottom_up(depth)) end
  print("trees", iters, depth, c)
end
print("long lived", check(long_lived))

local Vec = {}
Vec.__index = Vec
Vec.__add = function(a, b) return setmetatable({x = a.x + b.x, y = a.y + b.y}, Vec) end
Vec.__mul = function(a, k) return setmetatable({x = a.x * k, y = a.y * k}, Vec) end
Vec.__eq = function(a, b) return a.x == b.x and a.y == b.y end
function Vec.new(x, y) return setmetatable({x = x, y = y}, Vec) end
function Vec:len2() return self.x * self.x + self.y * self.y end
local acc = Vec.new(0, 0)
for i = 1, 300000 do acc = acc + Vec.new(i % 7, i % 11) * 2 end
print("vec", acc.x, acc.y, acc:len2(), acc == Vec.new(acc.x, acc.y))

local function counter()
  local n = 0
  return function(k) n = n + k; return n end
end
local fs = {}
for i = 1, 1000 do fs[i] = counter() end
local total = 0
for r = 1, 300 do for i = 1, 1000 do total = total + fs[i](r) end end
print("closures", total)

local gen = coroutine.wrap(function()
  local a, b = 0, 1
  for _ = 1, 200000 do
    coroutine.yield(a)
    a, b = b, (a + b) % 1000000007
  end
end)
local fsum = 0
for _ = 1, 200000 do fsum = (fsum + gen()) % 1000000007 end
print("coroutine", fsum)

local arr = {}
for i = 1, 200000 do arr[i] = math.random(1, 1000000) end
table.sort(arr)
local ok = true
for i = 2, #arr do if arr[i - 1] > arr[i] then ok = false end end
print("sort", ok, arr[1], arr[100000], arr[#arr])
local recs = {}
for i = 1, 50000 do recs[i] = {key = math.random(1, 500), id = i} end
table.sort(recs, function(a, b) if a.key ~= b.key then return a.key < b.key end return a.id < b.id end)
print("records", recs[1].key, recs[1].id, recs[50000].key, recs[50000].id)
