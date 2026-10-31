-- Enteros y flotantes: subtipos, división, desbordamiento, conversiones, bits y math.
print(math.type(1), math.type(1.0), math.type("1"), 1 == 1.0)
print(7 // 2, -7 // 2, 7 % -3, -7 % 3, 7.5 // 2, -7.5 % 2)
print(7 / 2, 2^10, 10 / 2, 3 % math.huge, -3 % math.huge)
print(math.maxinteger, math.mininteger, math.maxinteger + 1 == math.mininteger)
print(math.maxinteger * 2, math.mininteger // -1)
print(1e308 * 10, -1e308 * 10, math.huge // 1)
print(0x7fffffffffffffff, 0xffffffffffffffff, 0x10, 0xA.8p1)
print(5 & 3, 5 | 3, 5 ~ 3, ~5, 1 << 63, 1 << 64, -1 >> 1, 3 >> -1)
print(tostring(1/0), tostring(-1/0), (0/0) ~= (0/0))
print(math.tointeger(3.0), math.tointeger(3.5), math.tointeger("8"), math.tointeger(2^63))
print(tonumber("0x1p4"), tonumber("  12  "), tonumber("1e2"), tonumber("z", 36), tonumber("777", 8), tonumber("1.5e"))
print(3 | 0, 2^53 + 1, math.floor(-3.5), math.ceil(-3.5), math.floor(2^62) == 2^62)
print(math.abs(math.mininteger), math.abs(-0.0), math.fmod(7, -3), math.fmod(-7, 3), math.fmod(-7.5, 2))
print(math.max(3, 7.5, -1), math.min(3, 7.5, -1), math.ult(1, -1), math.ult(-1, 1))
print(string.format("%.17g %.17g %.17g", math.pi, math.exp(1), math.sqrt(2)))
print(string.format("%.15g %.15g %.15g", math.sin(1), math.cos(1), math.tan(1)))
print(string.format("%.15g %.15g %.15g", math.log(10), math.log(8, 2), math.log(1000, 10)))
print(string.format("%.15g %.15g", math.atan(1, 2), math.atan(-1)))
print(1e15, 1e16, 123456789012345678, 0.1 + 0.2, 100 / 3, -0.0)
print(2^63 == math.maxinteger + 1.0, math.maxinteger + 0.0 == 2^63, 2^53 == 2^53 + 1)
print(string.format("%d %5.1f %x %X %o %e %g %a", 42, 3.14159, 255, 255, 8, 12345.678, 0.0001, 1.0))
print(8 // 0.0, -8 // 0.0, pcall(function() return 1 // 0 end))
print(pcall(function() return 1 % 0 end))
print(10 == "10", "10" + 1, "3" * "4", "0x10" + 0, 10 .. 20)

-- Suma de Kahan frente a la ingenua.
local naive, sum, c = 0.0, 0.0, 0.0
for i = 1, 100000 do
  local x = 1 / i
  naive = naive + x
  local y = x - c
  local t = sum + y
  c = (t - sum) - y
  sum = t
end
print(string.format("%.17g %.17g", naive, sum))

-- Aritmética entera envolvente: un hash FNV-1a de 64 bits.
local h = -3750763034362895579
for i = 1, 1000 do
  h = h ~ (i & 0xff)
  h = h * 1099511628211
end
print(h, string.format("%016x", h))

-- Criba de primos.
local n, sieve = 10000, {}
for i = 2, n do sieve[i] = true end
for i = 2, math.floor(math.sqrt(n)) do
  if sieve[i] then for j = i * i, n, i do sieve[j] = false end end
end
local count, last = 0, 0
for i = 2, n do if sieve[i] then count = count + 1 last = i end end
print(count, last)
