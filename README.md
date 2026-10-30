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
| K1 | Núcleo y arnés: carga de ELF estáticos sin permiso de ejecución en el invitado, decodificador table-driven del alcance (`isa/scope.tsv`), JIT por recodificación con caché de código y encadenamiento, CPUID filtrado a x86-64-v2, llamadas al sistema básicas; arnés con generador aleatorio, comparación contra el CPU con máscara de flags indefinidas, mutantes y reductor ddmin; scripts `random_compare.py`, `coverage.py`, `mutants.py`, `reduce_check.py`, `qemu_tcg.py` (`test-x86_64`, `sha512-sse`) y los demás de K1 en la sección 6 del contrato | Implementado; sin verificar hasta el cierre del K |
| K2 | Sistema: llamadas al sistema, señales, hilos, código automodificable, ELF dinámicos | Pendiente |
| K3 | Programas reales y rendimiento: busybox, SQLite, Lua, TCC, medición contra qemu-x86_64 | Pendiente |

## Resultados

Todavía no hay resultados medidos. Cada K añade aquí los suyos al cerrar.
