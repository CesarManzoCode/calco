# Límites de calco

Fuente única de lo que calco no demuestra o no hace. Ningún otro archivo los repite.

## Alcance

- Solo x86-64 en modo usuario de Linux. Fuera de alcance: AVX y extensiones posteriores (VEX, EVEX, XOP), modo de 32 bits e `int 0x80`, modo sistema, Wine y programas gráficos.
- CPUID anuncia solo el nivel x86-64-v2 sin XSAVE. Una instrucción que el CPU de la máquina sí ejecuta pero que no está anunciada ni en la tabla del alcance (por ejemplo AVX, BMI, MOVBE, `rdfsbase`) da SIGILL bajo calco aunque en nativo funcione.
- `rep bsf` y `rep bsr` (`F3 0F BC/BD`) se ejecutan como TZCNT/LZCNT, igual que en el CPU anfitrión, aunque CPUID no anuncie esas extensiones.
- Los saltos y llamadas cercanos con prefijo `66` dan SIGILL.
- El retorno lejano (`retf`, `CA`/`CB`) termina con SIGSEGV, aunque en nativo un retorno lejano al selector de código de usuario funciona.
- Las instrucciones cuyo prefijo de segmento afecta a un operando de memoria implícito (`movs`, `cmps`, `lods`, `xlat`, `maskmovq`, `maskmovdqu`) dan SIGILL si llevan prefijo GS.

## Exactitud

- La exactitud de instrucción se demuestra contra un solo CPU (AMD Ryzen 5 5600G, Zen 3). En un CPU de otro fabricante o generación, las flags indefinidas y algunas decodificaciones pueden diferir; nada se midió fuera de esa máquina.
- La comparación aleatoria aplica una máscara de flags indefinidas tomada del manual; el modo estricto es solo informativo.
- Del estado x87 no se comparan FIP, FCS, FDP, FDS ni FOP: el puntero de la última instrucción x87 apunta a la caché de código.
- RDTSC, RDTSCP, RDRAND, RCPPS, RCPSS, RSQRTPS, RSQRTSS y CPUID quedan fuera del generador aleatorio; solo se comprueba que corren sin error.

## Sistema

- No hay protección del traductor frente al invitado: el espacio de direcciones se aísla por colocación, así que una escritura salvaje del invitado puede corromper a calco.
- `MAP_FIXED` sobre memoria de calco devuelve `ENOMEM`; `brk` está limitado a 1 GiB.
- `/proc/self/maps` del invitado muestra el mapa de calco, no uno filtrado.
- No se entrega vDSO al invitado; las llamadas de tiempo van al kernel.
- Las señales 62–64 del invitado no están disponibles; la 62 del anfitrión la usa calco.
- `vfork` se emula como `fork`; `clone` con `CLONE_VM` sin `CLONE_THREAD` no está soportado.
- Un hilo que está ejecutando un bloque cuando otro hilo modifica ese código lo termina con la versión vieja.
- Llamadas al sistema fuera de la lista del contrato devuelven `ENOSYS`.

## Mediciones

- Todas las mediciones son de una sola máquina, sin root, sin aislar el sistema más allá de fijar un núcleo.
- La comparación con nativo es un costo de traducción, no una comparación de velocidad entre iguales.
- `testthread` se compara por hilo y como multiconjunto de líneas, porque su orden global no es estable ni en nativo.
