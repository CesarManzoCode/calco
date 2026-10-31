-- string.format en todas sus conversiones, con anchos, precisiones y banderas.
local ints = {0, 1, -1, 42, -42, 255, 65535, math.maxinteger, math.mininteger}
for _, v in ipairs(ints) do
  print(string.format("[%d] [%5d] [%-5d] [%05d] [%+d] [% d] [%x] [%#x] [%X] [%o] [%#o]",
    v, v, v, v, v, v, v, v, v, v, v))
end
local floats = {0.0, -0.0, 1.5, -1.5, 3.14159265358979, 1e-10, 6.02214076e23, 1/3, 2^-1074, 1.7976931348623157e308}
for _, v in ipairs(floats) do
  print(string.format("[%f] [%.3f] [%10.2f] [%-10.2f] [%e] [%.2E] [%g] [%.10g] [%G] [%a] [%A]",
    v, v, v, v, v, v, v, v, v, v, v))
end
print(string.format("[%f] [%e] [%g]", 1/0, -1/0, 1/0))
print(string.format("[%s] [%10s] [%-10s] [%.3s]", "text", "text", "text", "text"))
print(string.format("[%s] [%s] [%s] [%s]", 12, 1.25, true, nil))
print(string.format("[%c%c%c] [%%] [%5.1s]", 76, 117, 97, "abc"))
print(string.format("%q", 1/3), string.format("%q", math.mininteger), string.format("%q", 255))
print(string.format("%q", "tab\there\r\n\\ \1\127 é"))
print(string.format("%i|%u", 7, 9 == 9 and 9 or 0))
print(pcall(string.format, "%d", 1.5))
print(pcall(string.format, "%d", "x"))
print(string.format("%.0f %.0f %.0f %.0f", 0.5, 1.5, 2.5, -0.5))
print(string.format("%5.2f%%", 99.567))
print(string.format("%-8s|%8s|", "left", "right"))
print(string.format("%3d|%-3d|%03d", 7, 7, 7))
print(string.format("%.20f", 0.1))
print(string.format("%.99f", 1/3):sub(1, 40))
print(string.format("%099d", 7):sub(90))
local rows = {{"alpha", 1, 0.5}, {"beta", 22, 12.25}, {"gamma", 333, 1234.125}}
for _, r in ipairs(rows) do print(string.format("%-6s %4d %10.3f %8.2e", r[1], r[2], r[3], r[3])) end
print(tostring(12), tostring(12.0), tostring(-0.0), tostring(1e100), tostring(2^63), tostring(1e15 + 0.5))
