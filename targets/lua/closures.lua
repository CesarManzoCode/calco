-- Cierres: contadores, upvalues compartidos, memoización, composición y recursión.
local function counter(start, step)
  local n = start
  return function() n = n + step return n end, function() return n end
end
local inc, get = counter(10, 3)
inc() inc() inc()
print(get())

local fns = {}
for i = 1, 5 do fns[i] = function() return i * i end end
local out = {}
for i = 1, 5 do out[i] = fns[i]() end
print(table.concat(out, " "))

local shared = {}
do
  local x = 0
  shared.add = function(d) x = x + d end
  shared.get = function() return x end
end
shared.add(5) shared.add(7)
print(shared.get())

local calls = 0
local function memoize(f)
  local cache = {}
  return function(n)
    local v = cache[n]
    if v == nil then v = f(n) cache[n] = v end
    return v
  end
end
local fib
fib = memoize(function(n) calls = calls + 1 if n < 2 then return n end return fib(n - 1) + fib(n - 2) end)
print(fib(90), calls)

local function compose(...)
  local fs = {...}
  return function(x)
    for i = #fs, 1, -1 do x = fs[i](x) end
    return x
  end
end
local f = compose(function(x) return x + 1 end, function(x) return x * 2 end, math.abs)
print(f(-5), f(7))

local function curry(f, n, args)
  args = args or {}
  return function(x)
    local a = {table.unpack(args)}
    a[#a + 1] = x
    if #a == n then return f(table.unpack(a)) end
    return curry(f, n, a)
  end
end
print(curry(function(a, b, c) return a * 100 + b * 10 + c end, 3)(1)(2)(3))

local function ackermann(m, n)
  if m == 0 then return n + 1 end
  if n == 0 then return ackermann(m - 1, 1) end
  return ackermann(m - 1, ackermann(m, n - 1))
end
print(ackermann(2, 3), ackermann(3, 4))

-- Recursión de cola profunda.
local function loop(n, acc) if n == 0 then return acc end return loop(n - 1, acc + n) end
print(loop(1000000, 0))

-- pcall, error con tabla y varargs.
local ok, err = pcall(function(...) error({code = select("#", ...)}) end, 1, 2, 3)
print(ok, err.code)
print(pcall(error, "plain", 0))
print(select(2, xpcall(function() return nil + 1 end, function(m) return "handled: " .. tostring(m):match("arithmetic") end)))
