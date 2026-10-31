-- Carga de rendimiento (contrato 8): norma espectral y n-cuerpos, aritmética de flotantes.
local function A(i, j)
  local ij = i + j - 1
  return 1.0 / (ij * (ij - 1) * 0.5 + i)
end

local function Av(x, y, N)
  for i = 1, N do
    local a = 0
    for j = 1, N do a = a + x[j] * A(i, j) end
    y[i] = a
  end
end

local function Atv(x, y, N)
  for i = 1, N do
    local a = 0
    for j = 1, N do a = a + x[j] * A(j, i) end
    y[i] = a
  end
end

local function AtAv(x, y, t, N) Av(x, t, N); Atv(t, y, N) end

local N = 300
local u, v, t = {}, {}, {}
for i = 1, N do u[i] = 1 end
for _ = 1, 10 do AtAv(u, v, t, N); AtAv(v, u, t, N) end
local vBv, vv = 0, 0
for i = 1, N do
  local ui, vi = u[i], v[i]
  vBv = vBv + ui * vi; vv = vv + vi * vi
end
print(string.format("spectral %0.9f", math.sqrt(vBv / vv)))

local PI = math.pi
local SOLAR_MASS = 4 * PI * PI
local DAYS = 365.24
local bodies = {
  {x = 0, y = 0, z = 0, vx = 0, vy = 0, vz = 0, mass = SOLAR_MASS},
  {x = 4.84143144246472090e+00, y = -1.16032004402742839e+00, z = -1.03622044471123109e-01,
   vx = 1.66007664274403694e-03 * DAYS, vy = 7.69901118419740425e-03 * DAYS,
   vz = -6.90460016972063023e-05 * DAYS, mass = 9.54791938424326609e-04 * SOLAR_MASS},
  {x = 8.34336671824457987e+00, y = 4.12479856412430479e+00, z = -4.03523417114321381e-01,
   vx = -2.76742510726862411e-03 * DAYS, vy = 4.99852801234917238e-03 * DAYS,
   vz = 2.30417297573763929e-05 * DAYS, mass = 2.85885980666130812e-04 * SOLAR_MASS},
  {x = 1.28943695621391310e+01, y = -1.51111514016986312e+01, z = -2.23307578892655734e-01,
   vx = 2.96460137564761618e-03 * DAYS, vy = 2.37847173959480950e-03 * DAYS,
   vz = -2.96589568540237556e-05 * DAYS, mass = 4.36624404335156298e-05 * SOLAR_MASS},
  {x = 1.53796971148509165e+01, y = -2.59193146099879641e+01, z = 1.79258772950371181e-01,
   vx = 2.68067772490389322e-03 * DAYS, vy = 1.62824170038242295e-03 * DAYS,
   vz = -9.51592254519715870e-05 * DAYS, mass = 5.15138902046611451e-05 * SOLAR_MASS},
}

local function energy()
  local e = 0
  for i = 1, #bodies do
    local b = bodies[i]
    e = e + 0.5 * b.mass * (b.vx * b.vx + b.vy * b.vy + b.vz * b.vz)
    for j = i + 1, #bodies do
      local b2 = bodies[j]
      local dx, dy, dz = b.x - b2.x, b.y - b2.y, b.z - b2.z
      e = e - b.mass * b2.mass / math.sqrt(dx * dx + dy * dy + dz * dz)
    end
  end
  return e
end

local function advance(dt)
  local n = #bodies
  for i = 1, n do
    local bi = bodies[i]
    for j = i + 1, n do
      local bj = bodies[j]
      local dx, dy, dz = bi.x - bj.x, bi.y - bj.y, bi.z - bj.z
      local d2 = dx * dx + dy * dy + dz * dz
      local mag = dt / (d2 * math.sqrt(d2))
      local bm, bjm = bi.mass * mag, bj.mass * mag
      bi.vx = bi.vx - dx * bjm; bi.vy = bi.vy - dy * bjm; bi.vz = bi.vz - dz * bjm
      bj.vx = bj.vx + dx * bm; bj.vy = bj.vy + dy * bm; bj.vz = bj.vz + dz * bm
    end
  end
  for i = 1, n do
    local b = bodies[i]
    b.x = b.x + dt * b.vx; b.y = b.y + dt * b.vy; b.z = b.z + dt * b.vz
  end
end

local px, py, pz = 0, 0, 0
for _, b in ipairs(bodies) do
  px = px + b.vx * b.mass; py = py + b.vy * b.mass; pz = pz + b.vz * b.mass
end
bodies[1].vx = -px / SOLAR_MASS; bodies[1].vy = -py / SOLAR_MASS; bodies[1].vz = -pz / SOLAR_MASS
print(string.format("nbody %0.9f", energy()))
for _ = 1, 200000 do advance(0.01) end
print(string.format("nbody %0.9f", energy()))
