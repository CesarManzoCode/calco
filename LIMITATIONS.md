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
- El generador aleatorio no usa AH, BH, CH ni DH como operandos explícitos (solo en la preparación de `div` de 8 bits y, implícitos, en `lahf`/`sahf`), ni `push`/`pop` de 16 bits, ni prefijos de segmento.
- La máscara de flags es conservadora en las instrucciones con cuenta (desplazamientos, rotaciones, `shld/shrd`, cadenas con `rep`): como la cuenta puede ser 0, las flags que escriben no se dan por definidas, y las que el manual deja indefinidas en algún caso se enmascaran siempre. La única excepción es `shl/shr/sar` con cuenta inmediata que, enmascarada como la enmascara el CPU, queda entre 1 y el ancho menos 1: ahí CF, PF, ZF y SF (y OF si la cuenta es 1) se dan por definidas; con cuenta en CL se sigue enmascarando todo.
- RDTSC, RDTSCP, RDRAND, RCPPS, RCPSS, RSQRTPS, RSQRTSS y CPUID quedan fuera del generador aleatorio; solo se comprueba que corren sin error.
- La comparación aleatoria no prueba fallas: el generador construye cada caso para que no falle (divisor no nulo, memoria dentro del búfer, excepciones x87 y SSE enmascaradas). Las excepciones de CPU y las señales que producen no se comparan contra el CPU en K1.
- La cobertura cuenta mnemónicos de `isa/scope.tsv` (470 de 470 al cerrar K1), no cada forma de operandos ni cada codificación de un mnemónico.
- Que el arnés detecte fallos se demuestra solo con los 6 mutantes fijos, cada uno de una línea en `emit/` o `jit/`. La reducción a 3 instrucciones o menos se demuestra solo sobre sus contraejemplos.
- El número de discrepancias de qemu-x86_64 y el resultado del modo estricto no quedaron registrados al cerrar K1.
- Señales, hilos, código automodificable y `execve` se demostraron solo con invitados estáticos: `guests/signals.c` (no-PIE), `guests/threads.c`, `guests/smc.c`, `guests/exec.c`, y `linux-test` y `testthread` de QEMU. Los invitados dinámicos comprobados son `guests/hello.c`, SQLite, Lua y TCC (salida) y `guests/pause.c` (permisos).
- La entrega de señales con estado exacto se demuestra solo con los cuatro casos de `guests/signals.c`: SIGSEGV por escritura con corrección de contexto, SIGILL por `ud2`, SIGFPE por división entre cero y SIGALRM de `setitimer` en un bucle de solo registros.
- Los hilos se demuestran con 50 corridas seguidas de `guests/threads.c` y una de `testthread` en una sola máquina. Una carrera que no aparezca en esas corridas no queda descartada.
- El código automodificable se demuestra con un solo hilo (`guests/smc.c`). No hay prueba de un hilo que modifica código que otro está ejecutando.
- La ausencia de `x` en un invitado dinámico se comprueba, por nombre, solo para el ejecutable, `ld-linux-x86-64.so.2` y `libc.so.6`. Otras bibliotecas que cargue el invitado no las revisa ninguna prueba.
- De busybox se comparan con nativo solo los 64 casos de `targets/busybox/cases.tsv` (55 applets de las 401 que trae el binario), y `tc` no se compila (`targets/busybox/config.notes`).
- La suite propia de busybox se compara solo en el subconjunto que pasa en nativo dos veces seguidas sin root ni red (903 pruebas en el cierre de K3) y con una sola corrida bajo calco. Lo que en nativo da `FAIL` (24), `SKIPPED` (6) o `UNTESTED` (1) queda fuera de la comparación.
- SQLite se demuestra solo con una carga SQL fija (`targets/sqlite/workload.sql`); su suite TCL no se corre.
- Lua se demuestra solo con los 9 scripts de `targets/lua/*.lua`.
- La autocompilación de TCC compara el sha256 del `tcc1` que produce `tcc0` en nativo y bajo calco; ese `tcc1` no se ejecuta ni se hace una segunda etapa.
- Cada programa objetivo se compara en una sola corrida por criterio; una diferencia que no aparezca en esa corrida no queda descartada.

## Sistema

- No hay protección del traductor frente al invitado: el espacio de direcciones se aísla por colocación, así que una escritura salvaje del invitado puede corromper a calco.
- `MAP_FIXED` sobre memoria de calco devuelve `ENOMEM`; `brk` está limitado a 1 GiB.
- `/proc/self/maps` del invitado muestra el mapa de calco, no uno filtrado.
- No se entrega vDSO al invitado; las llamadas de tiempo van al kernel.
- Los programas dinámicos usan el intérprete y las bibliotecas del anfitrión que nombre su `PT_INTERP`; no hay raíz alternativa del invitado (como `-L` de qemu-user).
- Las señales 62–64 del invitado no están disponibles; la 62 del anfitrión la usa calco.
- `vfork` se emula como `fork`; `clone` con `CLONE_VM` sin `CLONE_THREAD` no está soportado.
- Un hilo (`clone` con `CLONE_THREAD`) tiene que pedir `CLONE_VM`, `CLONE_FS`, `CLONE_FILES` y `CLONE_SIGHAND`, porque un hilo del anfitrión comparte todo eso siempre; solo puede añadir `CLONE_SYSVSEM`, `CLONE_SETTLS`, `CLONE_PARENT_SETTID`, `CLONE_CHILD_SETTID`, `CLONE_CHILD_CLEARTID` y `CLONE_DETACHED`. Cualquier otra combinación devuelve `EINVAL`, aunque el kernel la aceptaría.
- `set_robust_list` se guarda por proceso, no por hilo: `get_robust_list` devuelve la lista del último hilo que la fijó.
- `fork` desde un invitado con varios hilos: calco toma antes sus candados (caché, proceso, lista de hilos), pero no los internos de la glibc del anfitrión; si otro hilo de calco está dentro de `malloc` en ese instante, el hijo puede quedarse bloqueado en su primera reserva de memoria.
- Al terminar un hilo del invitado no se liberan su pila alternativa de calco (64 KiB) ni su estado de señales, y se pierden las señales que calco ya tenía en cola para él.
- Vaciar la caché con varios hilos espera a que cada hilo salga de ella: se le vuelve a mandar la señal 62 del anfitrión cada 100 µs hasta que llega a una frontera de instrucción. Si la 62 interrumpe una llamada que bloquea, la llamada se reinicia como con cualquier señal que el invitado no maneja (con el tiempo original, ver arriba).
- Si la prueba de TF del arranque falla y llega una señal asíncrona en medio de una expansión mientras otro hilo tiene el candado de la caché, el bloque no se desenlaza: la señal se entrega cuando el hilo vuelva a salir al despachador.
- `execve` comprueba antes de reemplazar el proceso lo que el kernel comprueba del archivo (existencia, permiso de ejecución, archivo regular, formato y cadena de `#!`); si el calco nuevo falla después al cargar el programa, termina con código 125 en lugar de devolver el errno al invitado.
- `execveat` relativo a un descriptor o con `AT_EMPTY_PATH` pasa la ruta como `/proc/self/fd/N`: si el descriptor tiene `O_CLOEXEC`, el calco nuevo ya no puede abrirlo y termina con código 125.
- `execve` lleva al programa nuevo la máscara y las señales ignoradas a través de las del anfitrión, salvo SIGPIPE, que siempre empieza con la acción por defecto (el runtime de Rust la ignora antes de `main`), y salvo SIGSEGV, SIGBUS, SIGILL, SIGFPE y SIGTRAP, que calco nunca bloquea en el anfitrión y por eso nunca llegan bloqueadas.
- `pause` no está en la lista cerrada de llamadas y devuelve `ENOSYS`.
- Una llamada con tiempo límite (`nanosleep`, `clock_nanosleep`, `poll`, `select`, `epoll_wait`, `futex`…) interrumpida por una señal que el invitado no maneja se reinicia con el tiempo original, no con el que le faltaba.
- `signalfd4` traduce la máscara, pero las lecturas del descriptor dan los números del anfitrión: 64 y 63 para las señales 32 y 33 del invitado. `rt_sigtimedwait` y los marcos sí las traducen.
- El área de punto flotante del marco de señal es FXSAVE de 512 bytes, sin XSAVE (coherente con CPUID), así que el marco queda en otra dirección de la pila que en nativo, donde el kernel usa XSAVE.
- Si no se puede escribir el marco de una señal (pila inválida o sin `SA_RESTORER`), el proceso muere por SIGSEGV; el kernel en cambio fuerza un SIGSEGV que un manejador de SIGSEGV del invitado todavía podría atender.
- Mientras calco avanza con TF hasta la siguiente frontera, una instrucción `pushfq` del invitado guarda TF encendida, y un `popfq` que la apaga detiene el avance: la señal se entrega cuando el bloque sale al despachador.
- Del `siginfo_t` de una señal del anfitrión solo se traducen `si_signo` y el `si_status` de SIGCHLD; cualquier otro campo que lleve un número de señal (por ejemplo un `si_value` armado por el emisor) llega como lo da el kernel.
- Un hilo que está ejecutando un bloque cuando otro hilo modifica ese código lo termina con la versión vieja.
- Una página del invitado con traducciones y permiso de escritura queda sin escritura en el anfitrión: si el kernel escribe ahí por una llamada del invitado (por ejemplo `read` a un búfer en esa página), la llamada devuelve `EFAULT`; si calco mismo escribe ahí al emular una llamada, el proceso muere por SIGSEGV.
- Cualquier escritura en una página con traducciones, aunque sea a datos y no a código, invalida todos los bloques de la página; el espacio de los bloques invalidados en la caché no se recupera hasta el siguiente vaciado completo.
- `brk` que encoge sobre páginas que el invitado hizo ejecutables no invalida sus traducciones.
- Llamadas al sistema fuera de la lista del contrato devuelven `ENOSYS`.
- `mremap` con `MREMAP_DONTUNMAP` devuelve `EINVAL`, y solo acepta rangos dentro de un único intervalo del invitado.
- `shmat` sin dirección siempre coloca el segmento en la ventana del invitado. Si el invitado mueve un segmento con `mremap`, calco deja de verlo como segmento y `shmdt` en la dirección nueva devuelve `EINVAL` (en nativo lo desprende).
- `semget`, `semop`, `msgget` y el resto de IPC de System V que no es memoria compartida no están en la lista y devuelven `ENOSYS`.
- `mmap` ignora `MAP_32BIT`: la colocación sin dirección fija siempre usa la ventana del invitado.
- `readlink` solo reescribe `/proc/self/exe` y `/proc/thread-self/exe`; `/proc/<pid>/exe` muestra a calco.
- `prctl` solo acepta `PR_SET_NAME`, `PR_GET_NAME` y `PR_SET_PDEATHSIG`; el resto devuelve `EINVAL`.
- El nombre del proceso (`/proc/PID/comm`) es el último componente del `argv[0]` del invitado; el kernel usa el nombre del archivo ejecutado, que casi siempre coincide pero no siempre. `/proc/PID/cmdline` y `/proc/PID/exe` siguen mostrando a calco.
- Un invitado que ejecuta el binario de calco no lo corre traducido: calco lo ejecuta tal cual, y es ese calco el que traduce el programa que recibe.

## Mediciones

- Todas las mediciones son de una sola máquina, sin root, sin aislar el sistema más allá de fijar un núcleo.
- La comparación con nativo es un costo de traducción, no una comparación de velocidad entre iguales.
- En K1 el código de verificación era mayor que el del traductor (`python3 scripts/loc.py`: `traductor: 3927`, `verificacion: 4450`). En el cierre de K2 el criterio «arnés menor que traductor» pasa, pero los conteos de ese cierre no quedaron registrados.
- En K3 el código de verificación volvió a ser mayor que el del traductor (`traductor: 6725`, `verificacion: 7694`). Para cumplir `k1.loc` se recortó la verificación hasta `verificacion: 6711`, sin cambiar la forma de contar: primero duplicados y ayudantes repetidos, después pruebas y aserciones que repetían una rama ya probada. El margen es de 14 líneas, así que cada prueba nueva obliga a crecer el traductor o a recortar otra. Lo que se dejó de probar en unidad: el `rip` de la salida por SIGILL cuando la instrucción inválida va después de otra en el bloque (lo sigue probando de punta a punta `signals.py`), los números de señal de `ud2`, `int3` y `hlt` en `decode` (los prueba `jit/tests.rs`) y la mayoría de las filas x87 de la tabla de decodificación (D8–DF es una sola entrada).
- El rendimiento sale de una sola ejecución de `scripts/bench.py` (1 calentamiento y 5 medidas por carga y modo) en una sola máquina, contra un solo qemu-x86_64 (11.1.2).
- El umbral calco/qemu ≤ 0.5 se aplica a la media geométrica de las cinco cargas, con el mismo peso para todas, y no a cada carga: en el cierre de K3, sqlite queda en 0.4950.
- Dos cargas son muy cortas: sha512-sse tarda 0.7 ms en nativo y tcc 36 ms (medianas). `bench.py` mide el proceso entero y no separa la carga y el arranque de la ejecución, así que en esas cargas la razón incluye ese costo fijo (sha512-sse da calco/nativo 5.84).
- El `ru_maxrss` de `os.wait4` tiene un piso: en el cierre de K3, 60 de las 75 corridas dan exactamente 36 696 KiB, incluidas todas las de nativo y qemu-x86_64. Un valor en ese piso no mide al programa, así que la memoria de nativo, qemu y calco no se puede comparar entre sí; lo único demostrado es que calco no pasa de 1 GiB. [hipótesis] Linux conserva el máximo de RSS del proceso a través de `execve`, así que el hijo hereda el RSS del Python que lo lanzó; no se comprobó.
- `testthread` se compara por hilo y como multiconjunto de líneas, porque su orden global no es estable ni en nativo.
