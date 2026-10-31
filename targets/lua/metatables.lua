-- Metatablas: aritmética, comparación, __index/__newindex, __call, __len, __concat,
-- __close, __tostring y herencia.
local Vec = {}
Vec.__index = Vec
local function vec(x, y) return setmetatable({x = x, y = y}, Vec) end
Vec.__add = function(a, b) return vec(a.x + b.x, a.y + b.y) end
Vec.__sub = function(a, b) return vec(a.x - b.x, a.y - b.y) end
Vec.__mul = function(a, b)
  if type(a) == "number" then return vec(a * b.x, a * b.y) end
  if type(b) == "number" then return vec(a.x * b, a.y * b) end
  return a.x * b.x + a.y * b.y
end
Vec.__unm = function(a) return vec(-a.x, -a.y) end
Vec.__eq = function(a, b) return a.x == b.x and a.y == b.y end
Vec.__lt = function(a, b) return a:len2() < b:len2() end
Vec.__le = function(a, b) return a:len2() <= b:len2() end
Vec.__tostring = function(a) return "(" .. a.x .. "," .. a.y .. ")" end
Vec.__concat = function(a, b) return tostring(a) .. tostring(b) end
Vec.__len = function() return 2 end
function Vec:len2() return self.x * self.x + self.y * self.y end

local a, b = vec(1, 2), vec(3, 4)
print(tostring(a + b), tostring(a - b), a * b, tostring(2 * a), tostring(-b))
print(a == vec(1, 2), a ~= b, a < b, b <= a, #a, a .. b)

-- Herencia de dos niveles.
local Animal = {}
Animal.__index = Animal
function Animal.new(name, sound) return setmetatable({name = name, sound = sound}, Animal) end
function Animal:speak() return self.name .. " says " .. self.sound end
local Dog = setmetatable({}, {__index = Animal})
Dog.__index = Dog
function Dog.new(name) local d = Animal.new(name, "woof") return setmetatable(d, Dog) end
function Dog:fetch() return self.name .. " fetches" end
local d = Dog.new("rex")
print(d:speak(), d:fetch(), getmetatable(d) == Dog)

-- Tabla de solo lectura y registro de accesos.
local log = {}
local function readonly(t)
  return setmetatable({}, {
    __index = function(_, k) log[#log + 1] = "get " .. tostring(k) return t[k] end,
    __newindex = function(_, k) error("readonly " .. tostring(k), 2) end,
    __len = function() return #t end,
  })
end
local ro = readonly({10, 20, 30})
print(ro[2], #ro, pcall(function() ro[1] = 5 end))
print(table.concat(log, ";"))

-- Valores por defecto y __call.
local defaults = setmetatable({}, {__index = function(t, k) local v = k * 2 rawset(t, k, v) return v end})
print(defaults[21], rawget(defaults, 21), rawget(defaults, 22))
local callable = setmetatable({}, {__call = function(self, x, y) return x * y, self == nil end})
print(callable(6, 7))

-- __close con variables to-be-closed.
local closed = {}
do
  local function res(name) return setmetatable({}, {__close = function() closed[#closed + 1] = name end}) end
  local r1 <close> = res("first")
  local r2 <close> = res("second")
  closed[#closed + 1] = "body"
end
print(table.concat(closed, ","))

-- __index en cadena con rawequal y __mod/__idiv/__band.
local Num = {}
Num.__index = Num
Num.__mod = function(a, b) return a.v % b end
Num.__idiv = function(a, b) return a.v // b end
Num.__band = function(a, b) return a.v & b end
Num.__shl = function(a, b) return a.v << b end
local n = setmetatable({v = 1234}, Num)
print(n % 100, n // 7, n & 0xff, n << 4, rawequal(n, n), rawlen({1, 2}))

print(getmetatable("abc").__index == string, ("x"):upper())
