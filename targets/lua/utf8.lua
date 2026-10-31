-- utf8: char, charpattern, codes, codepoint, len, offset y validación.
local s = "añoñandú — ∑ 日本語 🎉!"
print(#s, utf8.len(s))
local cps = {}
for p, c in utf8.codes(s) do cps[#cps + 1] = p .. ":" .. c end
print(table.concat(cps, " "))
print(utf8.codepoint(s, 1, -1))
print(utf8.char(72, 228, 8364, 0x1F600, 0x10FFFF):byte(1, -1))
print(utf8.char(0x48, 0x49), utf8.char())
for i = -3, 6 do io.write(tostring(utf8.offset(s, i)), " ") end
print()
print(utf8.offset(s, 0, 3), utf8.offset(s, 1, 4))
print(utf8.len("\xff"), utf8.len("abc\xe4"), utf8.len(s, 3), utf8.len(s, 1, 4))
print(pcall(utf8.codepoint, "\xff"))
print(utf8.len("\xf4\x90\x80\x80"), utf8.len("\xf4\x90\x80\x80", 1, -1, true))
print(utf8.codepoint("\u{7FFFFFFF}", 1, 1, true))
local count = 0
for ch in s:gmatch(utf8.charpattern) do count = count + 1 end
print(count, utf8.charpattern == "[\0-\x7F\xC2-\xFD][\x80-\xBF]*")

-- Invertir por caracteres y contar bytes por longitud de secuencia.
local rev = {}
for _, c in utf8.codes(s) do table.insert(rev, 1, utf8.char(c)) end
print(table.concat(rev))
local bylen = {0, 0, 0, 0}
for _, c in utf8.codes(s) do
  local n = #utf8.char(c)
  bylen[n] = bylen[n] + 1
end
print(table.concat(bylen, " "))
print(("\u{48}\u{20AC}\u{1F600}"):byte(1, -1))
print(pcall(utf8.codes("ab\xffcd")))
