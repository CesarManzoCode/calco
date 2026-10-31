# calco

calco es un traductor binario dinámico de x86-64 a x86-64 en modo usuario de Linux, en la línea de `qemu-x86_64`. Carga ejecutables ELF reales (estáticos y dinámicos con glibc), traduce su código con un JIT a una caché de código propia, emula llamadas al sistema, señales e hilos, y nunca ejecuta directamente una página del invitado.

Su exactitud se demuestra con comparaciones externas: el propio CPU como oráculo sobre secuencias aleatorias de instrucciones, salida byte a byte igual a la ejecución nativa en programas reales (busybox, SQLite, Lua, la autocompilación de TCC y las pruebas x86 de QEMU) y tiempo medido contra `qemu-x86_64` y contra nativo en la misma máquina.

- Qué debe hacer y cómo está diseñado: [`vault/CONTRACT.md`](vault/CONTRACT.md).
- Fuentes y versiones fijadas: [`vault/RESEARCH.md`](vault/RESEARCH.md).
- Lo que el proyecto no demuestra: [`LIMITATIONS.md`](LIMITATIONS.md).
- Cómo funciona el JIT: [`vault/JIT.md`](vault/JIT.md).
- Cambios: [`CHANGELOG.md`](CHANGELOG.md).

## Requisitos

Linux x86-64 sin root, Rust estable, gcc, binutils y Python 3. Las versiones con las que se mide están en `vault/RESEARCH.md`. Los programas de terceros se descargan con versión y hash fijados en `build/` (fuera de git).

```sh
make check                          # formato y pruebas
cargo build --release
target/release/calco ./programa args...
```

## Estado

| Fase | Contenido | Estado |
| --- | --- | --- |
| K0 | Fundación: contrato, investigación, plan y comando de chequeo | Terminado |
| K1 | Núcleo y arnés: carga de ELF estáticos sin permiso de ejecución en el invitado, decodificador table-driven del alcance (`isa/scope.tsv`), JIT por recodificación con caché de código y encadenamiento, CPUID filtrado a x86-64-v2, llamadas al sistema básicas; arnés con generador aleatorio, comparación contra el CPU con máscara de flags indefinidas, mutantes y reductor ddmin; scripts `random_compare.py`, `coverage.py`, `mutants.py`, `reduce_check.py`, `qemu_tcg.py` (`test-x86_64`, `sha512-sse`) y los demás de K1 en la sección 6 del contrato | Cerrada |
| K2 | Sistema: tabla cerrada de llamadas al sistema con `execve` (calco se vuelve a ejecutar), `#!`, `fork` y memoria compartida de System V; señales síncronas y asíncronas con estado exacto y marco `rt_sigframe`; hilos con caché de código compartida y vaciado en punto seguro; código automodificable con páginas de código sin escritura e invalidación por rango; ELF dinámicos cargados por su `PT_INTERP`; calco enlazado estático; scripts `signals.py`, `threads.py`, `smc.py`, `dynamic.py` y `qemu_tcg.py` (`linux-test`, `testthread`) | Cerrada |
| K3 | Programas reales y rendimiento: busybox, SQLite, Lua y TCC compilados de fuente fijada; casos de busybox y su suite propia, carga SQL con hash de la base, scripts Lua y autocompilación de TCC comparados con nativo; búsqueda en línea de saltos indirectos, llamadas y retornos; medición contra nativo y contra qemu-x86_64 con cinco cargas fijas (`scripts/bench.py`, `build/bench/results.json`); diseño del JIT en [`vault/JIT.md`](vault/JIT.md); scripts `busybox_run.py`, `busybox_suite.py`, `sqlite_run.py`, `lua_run.py`, `tcc_selfcompile.py` y `bench.py` | En curso |

## Resultados

Cada K añade aquí los suyos al cerrar. Máquina: AMD Ryzen 5 5600G, Linux 7.2.11, usuario sin root.

### K1 (cerrada el 2026-10-30)

| Medición | Comando | Resultado |
| --- | --- | --- |
| Criterios de salida de K1 | `python3 vault/private/checks/check_k1.py` | 16/16 pasan, salida 0 (83.6 s) |
| Pruebas | `make check` | 73 pruebas unitarias de Rust, 2 de integración de Rust y 76 del arnés en Python, todas pasan |
| Secuencias aleatorias contra el CPU | `python3 scripts/random_compare.py` | 100 000 casos, `discrepancias: 0` (con máscara de flags indefinidas) |
| Mutantes detectados | `python3 scripts/mutants.py` | `mutantes detectados: 6/6`, 20 000 casos por mutante |
| Reducción de contraejemplos | `python3 scripts/reduce_check.py` | Cada mutante se reduce a 3 instrucciones o menos y sigue fallando |
| Cobertura del alcance | `python3 scripts/coverage.py` | `cobertura: 470/470` mnemónicos |
| `test-x86_64` de QEMU | `python3 scripts/qemu_tcg.py test-x86_64` | Salida idéntica en nativo y bajo calco |
| `sha512-sse` de QEMU | `python3 scripts/qemu_tcg.py sha512-sse` | Salida idéntica en nativo y bajo calco |
| Código del invitado sin permiso de ejecución | `python3 scripts/noexec.py` | Ninguna región del ELF invitado tiene `x` |
| CPUID sin AVX | `python3 scripts/cpuid_avx.py` | CPUID sin AVX, y `vaddps` muere por SIGILL |
| Tamaño de traductor y verificación | `python3 scripts/loc.py` | `traductor: 3927`, `verificacion: 4450` (criterio no cumplido en K1, diferido a K2) |

### K2 (cerrada el 2026-10-30)

Todas las filas salen de una corrida de `make check` y de `python3 vault/private/checks/check_k2.py`, `check_k0.py` y `check_k1.py`. La columna «Criterio» nombra el criterio del checker que dio cada resultado.

| Medición | Criterio | Resultado |
| --- | --- | --- |
| Criterios de salida de K2 | `check_k2.py` | 9/9 pasan, salida 0 (22.7 s) |
| Criterios de K0 y K1 (acumulativo) | `check_k0.py`, `check_k1.py` | 13/13 y 16/16 pasan, salida 0 |
| Pruebas | `make check` | 107 pruebas unitarias de Rust, 7 de integración de Rust y 76 del arnés en Python, todas pasan |
| `linux-test` de QEMU | `k2.linux_test` | Mismo código de salida, stdout y stderr en nativo y bajo calco |
| `testthread` de QEMU | `k2.testthread` | Misma secuencia de líneas por hilo y mismo multiconjunto de líneas que en nativo |
| Señales | `k2.signals` | `guests/signals.c` (SIGSEGV con corrección de contexto, SIGILL, SIGFPE, SIGALRM asíncrono) igual a nativo |
| Hilos | `k2.threads` | `guests/threads.c` igual a nativo en 50 de 50 corridas seguidas |
| Código automodificable | `k2.smc` | `guests/smc.c` igual a nativo |
| ELF dinámico con glibc | `k2.dynamic` | `guests/hello.c` dinámico (PIE) igual a nativo |
| Dinámico sin permiso de ejecución | `k2.dynamic_noexec` | Con `guests/pause.c` dinámico en espera, el ejecutable, `ld-linux-x86-64.so.2` y `libc.so.6` no tienen `x` |
| `execve` | `k2.exec` | `guests/exec.c` sale con 0 y su salida es igual a `guests/exec.expected`: el hijo también corre traducido |
| Tamaño de traductor y verificación | `k1.loc` | Verificación < traductor y `scripts/loc.py` sale con 0; los conteos de este cierre no quedaron registrados |

### K3 (abierta)

#### Rendimiento

Comando: `python3 scripts/bench.py`. Cada carga corre en los tres modos fijada al núcleo 2 con `taskset -c 2`, con 1 corrida de calentamiento descartada (que es también la que se comprueba igual a nativo) y 5 medidas. El tiempo es de pared (`time.perf_counter`) y la memoria es `ru_maxrss` de `os.wait4`; en las cargas de varios procesos (lua, gzip) el tiempo es la suma y la memoria el máximo. La tabla da la mediana de las 5 medidas y el resumen, la media geométrica de las razones de medianas. Los números salen de `build/bench/results.json`, que también registra la versión de qemu, el modelo de CPU, el kernel y el núcleo.

calco frente a qemu-x86_64 es una comparación equivalente: el mismo binario, la misma salida y ejecución traducida en modo usuario en ambos. calco frente a nativo es el costo de traducir, no una comparación de velocidad entre iguales.

| Carga | Qué corre | Nativo (s) | qemu-x86_64 (s) | calco (s) | `maxrss_kb` máx. de calco |
| --- | --- | --- | --- | --- | --- |
| sqlite | `sqlite3 -batch` con `targets/sqlite/bench.sql`, base nueva en cada corrida | sin medir | sin medir | sin medir | sin medir |
| lua | `lua` con cada `targets/lua/bench/*.lua` | sin medir | sin medir | sin medir | sin medir |
| tcc | autocompilación de TCC (`tcc0` sobre `tcc.c`) | sin medir | sin medir | sin medir | sin medir |
| gzip | `busybox gzip -9` de 16 MiB de `targets/busybox/gen_bench_input.py` y `busybox sha256sum` del resultado | sin medir | sin medir | sin medir | sin medir |
| sha512-sse | prueba `sha512-sse` de QEMU | sin medir | sin medir | sin medir | sin medir |

| Resumen | Campo de `results.json` | Umbral | Resultado |
| --- | --- | --- | --- |
| `calco/qemu` | `geomean_calco_qemu` | ≤ 0.5 | sin medir |
| `calco/nativo` | `geomean_calco_native` | (informativo) | sin medir |
| RSS de calco por corrida | `workloads.*.calco[*].maxrss_kb` | ≤ 1 048 576 KiB | sin medir |
