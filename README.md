# calco

calco es un traductor binario dinámico de x86-64 a x86-64 en modo usuario de Linux, en la línea de `qemu-x86_64`. Carga ejecutables ELF reales (estáticos y dinámicos con glibc), traduce su código con un JIT a una caché de código propia, emula llamadas al sistema, señales e hilos, y nunca ejecuta directamente una página del invitado.

Su exactitud se demuestra con comparaciones externas: el propio CPU como oráculo sobre secuencias aleatorias de instrucciones, salida byte a byte igual a la ejecución nativa en programas reales (busybox, SQLite, Lua, la autocompilación de TCC y las pruebas x86 de QEMU) y tiempo medido contra `qemu-x86_64` y contra nativo en la misma máquina.

- Qué debe hacer y cómo está diseñado: [`vault/CONTRACT.md`](vault/CONTRACT.md).
- Fuentes y versiones fijadas: [`vault/RESEARCH.md`](vault/RESEARCH.md).
- Lo que el proyecto no demuestra: [`LIMITATIONS.md`](LIMITATIONS.md).
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
| K2 | Sistema: llamadas al sistema, señales, hilos, código automodificable, ELF dinámicos | Pendiente |
| K3 | Programas reales y rendimiento: busybox, SQLite, Lua, TCC, medición contra qemu-x86_64 | Pendiente |

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
