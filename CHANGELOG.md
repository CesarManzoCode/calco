# Cambios

## Sin publicar

### K0 — fundación

- Contrato del traductor y del arnés en `vault/CONTRACT.md`: mapeo identidad con aislamiento por colocación, traducción por recodificación sin representación intermedia, flags reales del anfitrión vivas entre bloques, correspondencia 1:1 de registros con el estado del traductor en GS, caché de 256 MiB con vaciado total, CPUID limitado a x86-64-v2.
- Fuentes y versiones fijadas en `vault/RESEARCH.md`: qemu-x86_64 11.1.2, busybox 1.36.1, SQLite 3.54.1, Lua 5.5.1, TCC `e23dabed`, pruebas de QEMU en el commit `4fc49f46`.
- Umbrales: 100 000 secuencias aleatorias, 50 corridas de hilos, contraejemplos de hasta 3 instrucciones, RSS de hasta 1 GiB, calco/qemu ≤ 0.5.
- Comando único de chequeo `make check` y CI que lo corre.

### K1 — núcleo

- k1-01: crate `calco` sin dependencias; CLI `calco [--argv0 NOMBRE] [--] PROGRAMA [ARGS...]` (errores propios con código 125); llamadas al sistema crudas con `asm!`; mapa de intervalos del invitado con permisos lógicos, ventana de colocación, `MAP_FIXED` según el contrato, `brk` limitado a 1 GiB y reserva de la caché de código; cargador de ELF estáticos (`ET_EXEC` y `ET_DYN` sin `PT_INTERP`) con pila inicial y auxv; `struct Cpu` con offsets fijos. Tras cargar termina con `calco: jit not implemented`.
