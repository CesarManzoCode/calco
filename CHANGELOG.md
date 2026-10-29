# Cambios

## Sin publicar

### K0 — fundación

- Contrato del traductor y del arnés en `vault/CONTRACT.md`: mapeo identidad con aislamiento por colocación, traducción por recodificación sin representación intermedia, flags reales del anfitrión vivas entre bloques, correspondencia 1:1 de registros con el estado del traductor en GS, caché de 256 MiB con vaciado total, CPUID limitado a x86-64-v2.
- Fuentes y versiones fijadas en `vault/RESEARCH.md`: qemu-x86_64 11.1.2, busybox 1.36.1, SQLite 3.54.1, Lua 5.5.1, TCC `e23dabed`, pruebas de QEMU en el commit `4fc49f46`.
- Umbrales: 100 000 secuencias aleatorias, 50 corridas de hilos, contraejemplos de hasta 3 instrucciones, RSS de hasta 1 GiB, calco/qemu ≤ 0.5.
- Comando único de chequeo `make check` y CI que lo corre.
