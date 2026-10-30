# Cambios

## Sin publicar

### K0 — fundación

- Contrato del traductor y del arnés en `vault/CONTRACT.md`: mapeo identidad con aislamiento por colocación, traducción por recodificación sin representación intermedia, flags reales del anfitrión vivas entre bloques, correspondencia 1:1 de registros con el estado del traductor en GS, caché de 256 MiB con vaciado total, CPUID limitado a x86-64-v2.
- Fuentes y versiones fijadas en `vault/RESEARCH.md`: qemu-x86_64 11.1.2, busybox 1.36.1, SQLite 3.54.1, Lua 5.5.1, TCC `e23dabed`, pruebas de QEMU en el commit `4fc49f46`.
- Umbrales: 100 000 secuencias aleatorias, 50 corridas de hilos, contraejemplos de hasta 3 instrucciones, RSS de hasta 1 GiB, calco/qemu ≤ 0.5.
- Comando único de chequeo `make check` y CI que lo corre.

### K1 — núcleo

- k1-01: crate `calco` sin dependencias; CLI `calco [--argv0 NOMBRE] [--] PROGRAMA [ARGS...]` (errores propios con código 125); llamadas al sistema crudas con `asm!`; mapa de intervalos del invitado con permisos lógicos, ventana de colocación, `MAP_FIXED` según el contrato, `brk` limitado a 1 GiB y reserva de la caché de código; cargador de ELF estáticos (`ET_EXEC` y `ET_DYN` sin `PT_INTERP`) con pila inicial y auxv; `struct Cpu` con offsets fijos. Tras cargar termina con `calco: jit not implemented`.
- k1-03: codificador del anfitrión en `src/emit/` (recodificación de una `Insn` desde sus campos, con desplazamiento RIP-relativo nuevo o con operando `[temporal]`, y las formas que inserta el traductor); traducción de bloques en `src/jit/translate.rs` con fin en el primer control de flujo, `syscall`/`cpuid`/trampa, cambio de página de 4 KiB o 64 instrucciones, y tabla `pc_anfitrión → (rip, temporales)` por instrucción; caché de 256 MiB con tabla hash, stubs de salida y encadenamiento por parcheo atómico de rel32; trampolines de entrada y salida en `naked_asm!`; despachador que entra a la caché y atiende salidas. Saltos indirectos y `ret` salen siempre al despachador. Una instrucción inválida o trampa, o código sin permiso lógico de ejecución, termina el proceso con la señal que daría el kernel. Mínimos provisionales: `write`, `exit` y `exit_group` pasan directo y el resto de las llamadas da `-ENOSYS`; `cpuid` devuelve el del anfitrión sin filtrar (k1-04 los completa). Invitado `guests/asm/basic.S` y prueba `tests/jit_basic.rs`.
