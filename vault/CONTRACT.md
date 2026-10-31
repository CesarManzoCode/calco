# Contrato de calco

Este documento dice qué debe hacer calco y fija cada decisión de diseño. Lo escribió K0 el 2026-10-29. Los K1 en adelante lo ejecutan; no lo reabren salvo que una medición lo obligue, y entonces el cambio se registra en `CHANGELOG.md` con su razón.

Convención: **[hecho]** es algo observado en esta máquina o en una fuente de `vault/RESEARCH.md`; **[diseño]** es una decisión aceptada; **[hipótesis]** es algo que todavía no se ha comprobado y que un K concreto debe confirmar.

## 1. Qué es el sistema

calco es un traductor binario dinámico de x86-64 a x86-64 en modo usuario de Linux. Recibe un ELF x86-64 (estático, estático-PIE o dinámico con glibc) y sus argumentos, y lo ejecuta así:

1. Carga el ELF y, si lo pide, su intérprete (`ld-linux-x86-64.so.2`) en el espacio de direcciones del proceso, **sin permiso de ejecución** en ninguna página del invitado.
2. Decodifica cada instrucción del invitado a una forma estructurada y emite, en una caché de código propia, código nativo equivalente. Nunca salta a una página del invitado.
3. Emula las llamadas al sistema, las señales (síncronas y asíncronas), los hilos, `execve` y el código automodificable.
4. Anuncia por CPUID solo la ISA del alcance (sección 3.9). Una instrucción fuera del alcance produce SIGILL en el invitado.

Lo que demuestra cada criterio de la idea está en la sección 6. Los límites van solo en `LIMITATIONS.md`.

## 2. Estructura del repositorio de producto

| Ruta | Contenido |
| --- | --- |
| `Cargo.toml`, `src/`, `.cargo/config.toml` | El traductor: un solo crate Rust `calco`, binario `target/release/calco`. Sin dependencias externas (solo `std`). Enlazado estático con la glibc del anfitrión (`crt-static`), así las únicas copias de `ld-linux-x86-64.so.2` y `libc.so.6` en el proceso son las del invitado. |
| `src/**/tests.rs` | Pruebas unitarias de cada módulo (`#[cfg(test)] mod tests;`). Cuentan como verificación, no como traductor. |
| `tests/` | Pruebas de integración Rust que compilan con `cc` un invitado de `guests/` y lo corren bajo calco. |
| `guests/` | Programas C de prueba dirigida (hola, pausa, AVX, señales, hilos, SMC, exec) y sus salidas esperadas cuando no se comparan contra nativo. |
| `isa/scope.tsv` | La tabla de instrucciones del alcance (sección 4.2). |
| `harness/` | El arnés en Python 3 (solo biblioteca estándar): generador, constructor de ELF de prueba, ejecutores, comparador, reductor. Sus tablas `harness/*.tsv`. Sus pruebas `harness/test_*.py`. |
| `scripts/` | Los scripts de verificación de la sección 6, en Python 3 (solo biblioteca estándar), todos con `scripts/common.py`. |
| `targets/` | Recetas y entradas fijas de los programas objetivo (busybox, SQLite, Lua, TCC). |
| `build/` | Salidas, descargas y binarios de terceros. Ignorado por git. |
| `Makefile` | `make check`: el comando único de chequeo. |

Módulos del crate (nombres fijos para que los prompts se entiendan entre sí):

| Módulo | Responsabilidad |
| --- | --- |
| `main.rs` | CLI: `calco [--argv0 NOMBRE] [--] PROGRAMA [ARGS...]`. Código de salida y señal de muerte iguales a los del invitado. |
| `sys.rs` | Llamadas al sistema crudas del anfitrión con `asm!` (`syscall`), sin crates. |
| `mem.rs` | Mapa de memoria del invitado: intervalos propios del invitado, permisos lógicos, ventana de colocación, `brk`. |
| `loader.rs` | Carga de ELF, intérprete, pila inicial, auxv, `#!`. |
| `cpu.rs` | `struct Cpu` por hilo (sección 3.3), accesible desde el código traducido por GS. |
| `decode/` | Decodificador table-driven del alcance (`decode/table.rs`). |
| `emit/` | Codificador de instrucciones x86-64 del anfitrión. |
| `jit/` | Traducción de bloques, caché de código, encadenamiento, despachador, trampolines de entrada y salida, búsqueda de saltos indirectos, metadatos de reconstrucción de estado. |
| `cpuid.rs` | CPUID filtrado. |
| `syscall/` | Emulación de llamadas al sistema. |
| `signal/` | Acciones del invitado, manejador del anfitrión, marcos de señal del invitado, `rt_sigreturn`, llamadas seguras. |
| `thread.rs` | `clone`, hilos del invitado, `exit`. |
| `smc.rs` | Protección contra escritura de páginas traducidas e invalidación. |

## 3. Diseño del traductor

### 3.1 Espacio de direcciones

- [diseño] **Mapeo identidad.** Una dirección del invitado es la misma dirección en el anfitrión. Los punteros que el invitado pasa al kernel valen tal cual.
- [diseño] **Aislamiento por colocación, no por protección.** calco ocupa lo que el kernel le dé (PIE alrededor de `0x55…`, bibliotecas y montón cerca de `0x7f…`, pila cerca de `0x7ff…`). El invitado vive en direcciones bajas:
  - ELF `ET_EXEC`: en sus `p_vaddr`.
  - ELF `ET_DYN` (PIE o estático-PIE): base `0x0000_1000_0000_0000`.
  - `brk`: empieza en el final de página del segmento más alto del ejecutable; límite de 1 GiB.
  - Caché de código: 256 MiB reservados en `align_up(inicio_brk + 1 GiB, 2 MiB)`, para que el código traducido quede a menos de 2 GiB del ejecutable.
  - Ventana de colocación del invitado (intérprete, bibliotecas, `mmap` sin dirección fija): crece hacia arriba desde el final de la caché de código + 16 MiB, hasta `0x0000_3fff_0000_0000`.
  - Pila principal del invitado: 8 MiB con tope en `0x0000_4000_0000_0000`, más una página de guarda.
- [diseño] `mmap` con `MAP_FIXED` del invitado: si el rango es del invitado se reemplaza; si no, se intenta con `MAP_FIXED_NOREPLACE`, y si choca con memoria de calco se devuelve `-ENOMEM`.
- [diseño] Toda página del invitado se mapea en el anfitrión **sin `PROT_EXEC`**. `mem.rs` guarda los permisos lógicos que pidió el invitado; traducir desde una página sin permiso lógico de ejecución entrega SIGSEGV al invitado.
- [diseño] No se pasa el vDSO al invitado (`AT_SYSINFO_EHDR` no se emite), así glibc hace las llamadas al sistema de tiempo de verdad.

### 3.2 Carga de ELF y arranque

- [diseño] `loader.rs` lee `PT_LOAD`, `PT_INTERP`, `PT_PHDR`, `PT_GNU_STACK` y `PT_TLS` solo como datos (el TLS lo arma glibc). Soporta `ET_EXEC`, `ET_DYN` sin intérprete (estático-PIE) y con intérprete (K2). Un archivo que empieza con `#!` se carga como lo hace el kernel: intérprete, argumento opcional, ruta del script.
- [diseño] Pila inicial con el formato del ABI System V: `argc`, `argv`, `envp`, auxv, cadenas, `AT_RANDOM` (16 bytes de `getrandom`), alineada a 16. auxv emitido: `AT_PHDR AT_PHENT AT_PHNUM AT_PAGESZ AT_BASE AT_FLAGS AT_ENTRY AT_UID AT_EUID AT_GID AT_EGID AT_SECURE(0) AT_RANDOM AT_HWCAP AT_HWCAP2(0) AT_CLKTCK(100) AT_PLATFORM("x86_64") AT_EXECFN AT_MINSIGSTKSZ AT_NULL`. `AT_HWCAP` es CPUID.1:EDX ya filtrado (sección 3.9). Antes de entrar al invitado, calco nombra el proceso (`prctl(PR_SET_NAME)`) con el último componente del `argv[0]` del invitado.

### 3.3 Registros y estado

- [diseño] **Correspondencia 1:1.** Dentro del código traducido los 16 registros generales, RFLAGS, los XMM, el estado x87/MMX y MXCSR del invitado viven en los mismos registros del anfitrión. RSP del invitado es RSP del anfitrión.
- [diseño] **Registros del traductor fuera del banco general.** GS base del anfitrión apunta al `Cpu` del hilo, fijado una vez por hilo con `wrgsbase` ([hecho] `AT_HWCAP2 = 0x2`, FSGSBASE habilitado en esta máquina; `rdgsbase` funciona en modo usuario). FS base del anfitrión vale el `fs_base` del invitado mientras corre código traducido y el de calco mientras corre Rust; los trampolines lo cambian con `wrfsbase`.
- [diseño] `struct Cpu` (`#[repr(C)]`, offsets fijos): `gpr[16]`, `rip`, `rflags`, `fs_base`, `gs_base` del invitado, área FXSAVE de 512 bytes alineada a 16, `scratch[4]`, `host_rsp`, `host_fs`, `next_rip`, `exit_reason`, `pending_signals`, puntero al estado de señales del hilo, `tid`, `clear_child_tid`.
- [diseño] Temporales: cuando una expansión necesita un registro, elige uno que la instrucción no use, lo guarda en `gs:[scratch]` y lo restaura. Toda instrucción insertada por el traductor que conviva con flags vivas es neutral para flags (`mov`, `lea`, `jmp`, `jrcxz`, `xchg`).
- [diseño] La pila del invitado nunca la usa el traductor: no hay `push`/`call` insertados sobre ella (respeta la zona roja de 128 bytes). Los helpers en Rust corren en la pila del anfitrión (`gs:[host_rsp]`).

### 3.4 Decodificación

- [diseño] Decodificador table-driven en `decode/table.rs` para los mapas de un byte, `0F`, `0F38` y `0F3A`, con prefijos de grupo 1–4, REX y prefijos obligatorios `66/F2/F3`. Produce una `Insn` con: prefijos tal cual, REX, mapa, opcode, ModRM/SIB/desplazamiento, inmediato, longitud, clase (`Plain`, `Branch`, `Call`, `Ret`, `IndirectBranch`, `Syscall`, `Cpuid`, `Trap`, `Invalid`) y si usa RIP-relativo o segmento GS/FS.
- [diseño] El conjunto aceptado es exactamente `isa/scope.tsv` más las codificaciones heredadas que el hardware trata como otra instrucción del mismo opcode: `F3 0F BC/BD` (`rep bsf/bsr`, que el anfitrión ejecuta como TZCNT/LZCNT, igual que en nativo), `F3 C3` (`rep ret`), `endbr64` y el resto del espacio de NOP con pista `0F 18–1F`, y el prefijo `3E` (`notrack`) en saltos indirectos. VEX (`C4`/`C5`), EVEX (`62`), XOP y cualquier opcode fuera de la tabla son `Invalid` → SIGILL en el invitado con RIP en esa instrucción.
- [diseño] Donde Intel y AMD difieren en decodificación, se sigue al CPU de César (AMD Zen 3, sección 7). Los saltos y llamadas cercanos con prefijo `66` quedan fuera del alcance y son `Invalid`.
- [diseño] Longitud máxima 15 bytes; más es `Invalid`. Una instrucción que cruza a una página sin permiso lógico de ejecución da SIGSEGV con la dirección del primer byte inaccesible.

### 3.5 Emisión: traducción por recodificación

- [diseño] **Sin representación intermedia.** Cada `Insn` se vuelve a codificar con `emit/` a partir de sus campos decodificados (nunca copiando los bytes del invitado). Las instrucciones `Plain` se recodifican con los mismos prefijos, opcode y operandos, así que el anfitrión produce resultado, flags (también las indefinidas), excepciones y estado x87/SSE idénticos a los del CPU real. Las únicas reescrituras son:
  - RIP-relativo: si `destino − pc_anfitrión` cabe en rel32 se recodifica como RIP-relativo con el desplazamiento nuevo; si no, un temporal con `mov r, imm64` y el operando `[r]`.
  - Segmento GS del invitado: temporal con `gs_base` del invitado sumado con `lea`. Segmento FS: nativo, porque FS base ya es la del invitado.
  - Control de flujo: `jmp/jcc/loop/jrcxz/call/ret` (sección 3.7).
  - `syscall`, `cpuid`: salida al despachador o helper. `ud2`, `int3`, `int n`, `hlt`, privilegiadas: salida con la señal que daría el kernel (SIGILL, SIGTRAP, SIGSEGV).
- [diseño] Optimizaciones: solo encadenamiento, búsqueda en línea de saltos indirectos y colocación de la caché cerca del ejecutable. No hay trazas ni asignación de registros: la correspondencia 1:1 las vuelve innecesarias.
- [diseño] Un bloque termina en la primera instrucción de control de flujo, en `syscall`/`cpuid`/trampa, al cruzar 4 KiB de código del invitado o a las 64 instrucciones. Cada bloque guarda una tabla `pc_anfitrión → (rip_invitado, temporales en scratch)` por instrucción, que permite reconstruir el estado exacto del invitado (secciones 3.11 y 3.13).

### 3.6 Flags

- [diseño] **Flags reales del anfitrión, vivas entre bloques.** No hay flags perezosas: el resultado de una instrucción recodificada deja en RFLAGS exactamente lo que dejaría el CPU real. Las flags se conservan entre bloques encadenados porque todo el código insertado es neutral para flags; se guardan en `Cpu.rflags` (`pushfq` sobre la pila del anfitrión) solo al salir al despachador, y DF se limpia antes de entrar a Rust. Al volver, se restauran con `popfq`.
- [diseño] Consecuencia: las flags indefinidas coinciden con las del CPU de César sin modelarlas. La comparación del arnés usa máscara (sección 4.4); el modo estricto es solo informativo.

### 3.7 Caché de código, encadenamiento y saltos indirectos

- [diseño] Caché: arena de 256 MiB (sección 3.1), escrita con `PROT_READ|PROT_WRITE|PROT_EXEC` propia de calco. Tabla hash `rip_invitado → bloque` con direccionamiento abierto.
- [diseño] Expulsión: cuando la arena se llena se vacía completa (todos los hilos en punto seguro, sección 3.12) y se vuelve a empezar. No hay expulsión parcial.
- [diseño] Coherencia: las invalidaciones por página (SMC, `munmap`, `mprotect` que quita ejecución) desenlazan los bloques afectados y los sacan de la tabla. Cada bloque guarda sus enlaces entrantes.
- [diseño] Encadenamiento directo: cada salida de bloque es un `jmp rel32` (y `jcc rel32` + `jmp rel32` en saltos condicionales) a un stub de salida; al traducir el destino se parchea el rel32 (escritura atómica de 4 bytes alineada).
- [diseño] Saltos indirectos y `ret`: el destino del invitado (registro, memoria o `pop` de la pila del invitado) se lleva a `gs:[next_rip]`. En K1 se sale siempre al despachador. En K3 se añade la búsqueda en línea: una sonda en una tabla de correspondencia directa `rip → código` usando `lea` + `jrcxz` con RCX guardado en scratch, sin tocar flags; si falla, salida al despachador.
- [diseño] `call`: escribe la dirección de retorno del invitado en su pila (`lea rsp,[rsp-8]` y `mov [rsp], imm32` o temporal con `imm64`) y salta como un `jmp`. No hay pila de retorno sombra.

### 3.8 x87, MMX, SSE y MXCSR

- [diseño] Se recodifican de forma nativa. Precisión de 80 bits, palabra de control, tags, excepciones (incluidas las desenmascaradas que dan SIGFPE) y MXCSR se comportan como el hardware. En cada salida al despachador se guarda el estado con `fxsave64` en `Cpu` y Rust corre con FCW `0x037F` y MXCSR `0x1F80`; al entrar se restaura con `fxrstor64`.
- [diseño] FIP, FCS, FDP, FDS y FOP del área FXSAVE no se comparan (FIP apunta, por construcción, a la caché de código).

### 3.9 CPUID

- [diseño] `cpuid.rs` ejecuta el CPUID del anfitrión y filtra. Hojas con bits de función que se filtran: 1, 7 (todas las subhojas) y `0x80000001`. Hojas informativas sin bits de función que pasan tal cual: 0 (con la hoja máxima del anfitrión), 4, `0xB`, `0x80000000`, `0x80000002–4` (nombre), `0x80000005`, `0x80000006`, `0x80000008`, `0x8000001D`, `0x8000001E` (glibc las usa para tamaños de caché). Cualquier otra hoja, incluida `0xD` (XSAVE), devuelve ceros. Bits que quedan, si el anfitrión los tiene: hoja 1 EDX = FPU TSC CX8 CMOV MMX FXSR SSE SSE2 (y los informativos PSE, PAE, PGE, CLFSH); hoja 1 ECX = SSE3 PCLMULQDQ SSSE3 CX16 SSE4.1 SSE4.2 POPCNT RDRAND; hoja 7 EBX/ECX/EDX = 0; `0x80000001` ECX = LAHF/SAHF, EDX = SYSCALL NX RDTSCP LM.
- [diseño] Se borran explícitamente AVX, OSXSAVE, XSAVE, F16C, FMA, MOVBE, AES, AVX2, BMI1/2, ADX, AVX-512, SHA, FSGSBASE, CET (SHSTK/IBT), LZCNT/ABM y SSE4A. `arch_prctl(ARCH_SHSTK_*)` devuelve `-EINVAL`.
- [hecho] glibc 2.44 de esta máquina elige rutinas según el CPUID que ve: bajo `qemu-x86_64 -cpu Nehalem`, `ld.so --help` deja de listar `x86-64-v3` como soportado y `--list-diagnostics` da `AVX_Fast_Unaligned_Load=0x0`; en nativo da `x86-64-v3 (supported)` y `0x1`.

### 3.10 Llamadas al sistema

- [diseño] `syscall` sale al despachador; `syscall/` decide por número. Por defecto, **paso directo** al kernel con los mismos argumentos (el mapeo identidad hace válidos los punteros) mediante «llamada segura» (sección 3.11). Una llamada no listada devuelve `-ENOSYS` y, si `CALCO_LOG` nombra un archivo, se anota ahí.
- [diseño] Lista cerrada (nombres del ABI x86-64 de `/usr/include/asm/unistd_64.h`):
  - Paso directo: `read write readv writev pread64 pwrite64 preadv pwritev openat open close close_range lseek fstat stat lstat newfstatat statx statfs fstatfs access faccessat faccessat2 getdents64 getcwd chdir fchdir mkdir mkdirat mknod mknodat rmdir unlink unlinkat rename renameat renameat2 link linkat symlink symlinkat readlink(con /proc/self/exe) chmod fchmod fchmodat chown fchown fchownat lchown utimensat futimesat truncate ftruncate fsync fdatasync fallocate fadvise64 sync umask dup dup2 dup3 pipe pipe2 fcntl flock ioctl poll ppoll select pselect6 epoll_create1 epoll_ctl epoll_wait epoll_pwait eventfd2 timerfd_create timerfd_settime timerfd_gettime memfd_create copy_file_range sendfile splice socket socketpair connect bind listen accept accept4 sendto recvfrom sendmsg recvmsg shutdown getsockopt setsockopt getsockname getpeername getpid getppid gettid getuid geteuid getgid getegid getgroups setuid setgid setreuid setregid setresuid setresgid getresuid getresgid setpgid getpgid getpgrp setsid getsid getrusage times uname sysinfo prlimit64 getrlimit setrlimit getpriority setpriority sched_yield sched_getaffinity sched_setaffinity sched_getparam sched_getscheduler clock_gettime clock_getres clock_nanosleep nanosleep gettimeofday time getrandom futex wait4 waitid alarm setitimer getitimer timer_create timer_settime timer_gettime timer_delete prctl(PR_SET_NAME, PR_GET_NAME, PR_SET_PDEATHSIG) membarrier mlock munlock mincore madvise(salvo DONTNEED en páginas con traducciones, que además invalida) msync inotify_init1 inotify_add_watch inotify_rm_watch shmget shmctl`.
  - Emuladas: `brk mmap munmap mprotect mremap` (mem.rs, sin `PROT_EXEC`, invalidación), `arch_prctl` (`ARCH_SET_FS/GET_FS/SET_GS/GET_GS`; resto `-EINVAL`), `set_tid_address`, `set_robust_list`/`get_robust_list` (se guardan, no se pasan), `rseq` (`-ENOSYS`, glibc lo desactiva), `clone` y `clone3` (`clone3` → `-ENOSYS`, glibc cae a `clone`), `fork vfork` (como `fork`), `execve execveat`, `exit exit_group`, `rt_sigaction rt_sigprocmask rt_sigreturn rt_sigpending rt_sigsuspend rt_sigtimedwait rt_sigqueueinfo rt_tgsigqueueinfo sigaltstack kill tkill tgkill`, `readlink`/`readlinkat` de `/proc/self/exe`, `signalfd4` (máscara traducida), `shmat shmdt` (mem.rs: dirección pedida o ventana de colocación, sin `SHM_EXEC` en el anfitrión, invalidación al desprender).
- [diseño] `execve`: calco se vuelve a ejecutar a sí mismo (`/proc/self/exe` del anfitrión, resuelto al arrancar) con `--argv0 <argv[0]> -- <ruta> <args...>` y el entorno del invitado, así el hijo también corre traducido. Si la ruta es el propio binario de calco (mismo dispositivo e inodo), se ejecuta tal cual con el argv del invitado: el calco nuevo ya traduce el programa que recibe, y un calco dentro de otro pondría su invitado en las direcciones del invitado externo.
- [diseño] `/proc/self/maps` del invitado no se filtra: muestra el de calco.

### 3.11 Señales

- [diseño] **Números.** Señales 1–31 del invitado = las mismas del anfitrión. Las 32 y 33 del invitado (que la glibc del invitado usa internamente) se mapean a las 64 y 63 del anfitrión. Las 34–61 son iguales. La 62 del anfitrión la reserva calco para «patear» hilos (vaciar la caché, invalidar). Las 62–64 del invitado devuelven `-EINVAL` en `rt_sigaction`. Las máscaras y `kill/tgkill/sigqueue` se traducen.
- [diseño] calco instala un manejador propio (con `SA_SIGINFO|SA_ONSTACK`, pila alternativa por hilo) para toda señal que el invitado maneje y para SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGTRAP siempre. La entrada al manejador es un stub en ensamblador que restaura FS de calco antes de entrar a Rust.
- [diseño] **Síncronas** (falla de memoria, `ud2`, división entre cero, SIGFPE de x87/SSE, `int3`): el `pc` del `ucontext` está en el código traducido; con la tabla del bloque se obtiene el `rip` del invitado y los temporales en scratch se restauran. Como cada instrucción recodificada falla antes de tener efectos, el estado del `ucontext` corregido es el estado exacto del invitado.
- [diseño] **Asíncronas**: si llegan con el `pc` en Rust, quedan en `Cpu.pending_signals` y el despachador las entrega antes de entrar al siguiente bloque. Si llegan dentro de la caché de código en una frontera de instrucción del invitado, se entregan ya. Si caen en medio de una expansión, el manejador pone TF en el `ucontext` y avanza instrucción por instrucción del anfitrión (SIGTRAP) hasta la siguiente frontera; ahí quita TF y entrega. [hipótesis] El kernel acepta TF desde `rt_sigreturn` (`FIX_EFLAGS` lo incluye); si K2 lo refuta, el respaldo es desenlazar el bloque actual y entregar en su salida.
- [diseño] **Llamadas seguras**: las llamadas que bloquean se hacen desde un stub cuyo rango de instrucciones conoce el manejador; si la señal llega entre la comprobación de pendientes y la instrucción `syscall`, el manejador retrocede el `pc` a un camino que devuelve `-EINTR` interno y la llamada se reinicia o no según `SA_RESTART` del invitado.
- [diseño] **Marco del invitado**: `rt_sigframe` de x86-64 como lo arma el kernel (`/usr/include/asm/sigcontext.h`, `/usr/include/asm/ucontext.h`, `/usr/include/asm/siginfo.h`): `pretcode` = `sa_restorer`, `ucontext` con `mcontext` (gregs con el estado del invitado), máscara del invitado y `fpstate` en formato FXSAVE con `sw_reserved.magic1 = 0` (sin XSAVE, coherente con CPUID). Respeta `sigaltstack` y `SA_NODEFER/SA_RESETHAND`. `rt_sigreturn` lee el marco y continúa en el `rip` del invitado vía despachador.
- [diseño] Acción por defecto que mata: calco restablece la acción del anfitrión y se reenvía la señal, así el proceso muere por la misma señal que en nativo.

### 3.12 Hilos

- [diseño] Un hilo del invitado = un hilo `std::thread` del anfitrión con su propio `Cpu` y su GS. `clone` con `CLONE_VM|CLONE_THREAD`: el hijo empieza con los registros del padre, `rax = 0`, `rsp` = pila pedida y `fs_base` = `tls` si hay `CLONE_SETTLS`. `CLONE_PARENT_SETTID`/`CLONE_CHILD_SETTID` los escribe calco antes de que el padre continúe (sincronización con el hijo); `CLONE_CHILD_CLEARTID` lo emula calco al salir el hilo (escribe 0 y `futex(FUTEX_WAKE)`), sin tocar el `set_tid_address` de la glibc del anfitrión.
- [diseño] El `tid` del invitado es el `gettid` real del hilo del anfitrión. `futex` pasa directo.
- [diseño] Caché compartida: traducir, parchear e invalidar se hace bajo un `Mutex` global; la búsqueda en la tabla hash no toma el candado (las entradas se publican con escritura atómica). Vaciar la caché pide a todos los hilos un punto seguro con la señal 62 del anfitrión y una barrera.
- [diseño] `CLONE_VM` sin `CLONE_THREAD` (más allá de `vfork`) devuelve `-EINVAL`.

### 3.13 Código automodificable

- [diseño] Al traducir desde una página que el invitado puede escribir, calco le quita escritura en el anfitrión y la marca como «con traducciones». Una escritura del invitado ahí da SIGSEGV en calco; el manejador invalida los bloques de esa página, devuelve la escritura en el anfitrión y retoma en la frontera de instrucción del invitado reconstruida (sección 3.11), sin entregar nada al invitado. Si el bloque en curso quedó invalidado, retoma por el despachador.
- [diseño] Una escritura desde otro hilo invalida igual; el hilo que está dentro del bloque viejo lo termina (garantía igual a la del hardware sin serialización).

## 4. Arnés de verificación

### 4.1 Forma

- [diseño] El arnés no tiene ejecutor nativo propio: genera, por lotes, un **ELF de prueba** estático sin libc con `as` y `ld` de binutils. Ese mismo binario se corre en nativo, bajo calco y bajo `qemu-x86_64`, y cada uno escribe sus volcados por stdout. El CPU es el oráculo.
- [diseño] Cada caso del lote: cargar estado inicial (GPR, RFLAGS con `popfq`, x87/SSE/MXCSR con `fxrstor64`, búfer de datos), ejecutar la secuencia, `call dump`. `dump` guarda RFLAGS con `pushfq`, los GPR, `fxsave64`, copia la memoria y hace `write(1, registro, 1280)`. Al final del lote, `exit_group(0)`.
- [diseño] Lote = 500 casos. Semillas fijas: el caso `i` usa la semilla `i` (`random.Random(i)`), así cada caso se regenera solo.

### 4.2 Tablas

- `isa/scope.tsv`: columnas `mnemonic` (nombre de `objdump -M intel`), `ext` (`base`, `x87`, `mmx`, `sse`, `sse2`, `sse3`, `ssse3`, `sse4.1`, `sse4.2`), `forms` (plantillas de operandos), `flags_read`, `flags_written`, `flags_undefined`, `ref` (sección del manual). Es la definición del alcance.
- `harness/undefined_flags.tsv`: `mnemonic`, `undefined` (flags), `ref` (AMD64 APM Vol. 3, pub. 24594, y la sección; Intel SDM Vol. 2 como segunda referencia). La revisión del manual consultado va en la cabecera del archivo.
- `harness/exclusions.tsv`: `mnemonic`, `reason`. Contiene al menos `rdtsc rdtscp rdrand rcpps rcpss rsqrtps rsqrtss cpuid syscall` y las privilegiadas. RDSEED no está en el alcance (CPUID no lo anuncia). El generador no las emite.
- `harness/mutants.tsv`: `feature`, `description`.

### 4.3 Generador

- [diseño] Plantillas por mnemónico tomadas de `isa/scope.tsv`. Operandos aleatorios con todos los registros (salvo RSP como destino general), inmediatos con sesgo a bordes (0, 1, −1, máximos, potencias de 2), y memoria siempre dentro del búfer de 256 bytes: RIP-relativo, `[disp32]` absoluto con SIB sin base y `[base+índice*escala+disp]` con la base cargada antes por `lea` del búfer.
- [diseño] Longitud por caso: 1–16 instrucciones. Saltos solo hacia adelante dentro de la secuencia; `call 1f; 1: pop r` para observar RIP.
- [diseño] Sin fallas: `div/idiv` van precedidos de una preparación que garantiza divisor no nulo y cociente representable; MXCSR inicial con excepciones enmascaradas (redondeo, DAZ y FTZ aleatorios); x87 con `fninit`, FCW con excepciones enmascaradas y algunos valores cargados.
- [diseño] **Definición de flags**: el generador sigue, instrucción por instrucción, qué flags están definidas; solo emite una instrucción que lee flags (`adc sbb jcc setcc cmovcc pushfq lahf rcl rcr …`) si todas las que lee están definidas. La máscara del caso es el conjunto indefinido al final. Se escribe en un archivo lateral por lote.

### 4.4 Comparación

- [diseño] Registro de volcado (1280 bytes, little-endian): `0` magia `CALCODMP` (8) · `8` índice de caso u32 · `12` reservado u32 · `16` GPR ×16 en orden de codificación (128) · `144` RIP (8) · `152` RFLAGS (8) · `160` FXSAVE64 (512) · `672` búfer de datos (256) · `928` ventana de pila de 256 bytes bajo el RSP inicial · `1184` relleno con ceros (96).
- [diseño] Se comparan: GPR, RIP, RFLAGS con la máscara del caso (y siempre sin RF ni bits reservados), FCW, FSW, FTW abreviado, ST0–ST7, MXCSR, XMM0–XMM15, búfer y ventana de pila. No se comparan FIP/FCS/FDP/FDS/FOP ni `mxcsr_mask`.
- [diseño] **Máscara siempre; modo estricto informativo.** `--strict` compara también las flags indefinidas e imprime cuántas coinciden; no decide pasa/falla.

### 4.5 Reducción

- [diseño] ddmin sobre la lista de instrucciones (conservando las preparaciones de `div` junto a su instrucción), manteniendo fijo el estado inicial; después, simplificación de inmediatos hacia 0 mientras siga fallando. Umbral: contraejemplo reducido ≤ 3 instrucciones.

### 4.6 Mutantes

- [diseño] Fallos inyectados como *features* de Cargo del traductor, `mutant-<nombre>`, cada uno de una línea en `emit/` o `jit/`. Conjunto fijo M = 6: `mutant-adc-carry` (ADC se emite como ADD), `mutant-shift-flags` (tras SHL/SHR/SAR se emite `cmc`), `mutant-movsxd` (MOVSXD se emite como MOV de 32 bits), `mutant-riprel-disp` (desplazamiento RIP-relativo +1), `mutant-pshufd-imm` (inmediato de PSHUFD con el bit 0 invertido), `mutant-fadd-fsub` (FADD se emite como FSUB). Cada mutante se compila con `cargo build --release --features mutant-<nombre> --target-dir build/mutants/<nombre>`.

## 5. Programas objetivo

| Programa | Versión fijada | Origen | Construcción |
| --- | --- | --- | --- |
| busybox | 1.36.1 | `https://busybox.net/downloads/busybox-1.36.1.tar.bz2`, sha256 `b8cc24c9574d809e7279c3be349795c5d5ceb6fdf19ca709f80cde50e47de314` | `defconfig` con `CONFIG_STATIC=y`; las applets que no compilen con los encabezados del sistema se desactivan y se anotan en `targets/busybox/config.notes`. Estático. |
| SQLite | 3.54.1 | `https://www.sqlite.org/2026/sqlite-amalgamation-3540100.zip`, sha256 `0a9c8e34352dad46006aa82836d5383bff9dc6bc9ce5a60553c05aa30e6ad1f9` | `gcc -O2 shell.c sqlite3.c -lm`. Dinámico con la glibc del sistema. |
| Lua | 5.5.1 | `https://www.lua.org/ftp/lua-5.5.1.tar.gz`, sha256 `1c4b4068d67061f2a2231ad2b5422e77acea1487ea9890f6320af614f4373dce` | `make linux` sin readline (`MYLIBS=` vacío). Dinámico. |
| TCC | mob `e23dabed154a8495f3ac1ff3cc9d04fa1c207893` | `https://repo.or.cz/tinycc.git` (`git clone` + `checkout` + verificación de `rev-parse HEAD`) | `./configure && make` con gcc → `tcc0`. Dinámico. |
| Pruebas de QEMU | v11.1.2, commit `4fc49f46dc95d4a27de2509e7fceb2931e91faeb` | `https://gitlab.com/qemu-project/qemu/-/raw/<commit>/tests/tcg/…` | Sección 6, con los sha256 de `vault/RESEARCH.md`. |
| qemu-x86_64 | 11.1.2-1 (Arch `qemu-user-static`) | `https://geo.mirror.pkgbuild.com/extra/os/x86_64/qemu-user-static-11.1.2-1-x86_64.pkg.tar.zst`, respaldo `https://archive.archlinux.org/packages/q/qemu-user-static/qemu-user-static-11.1.2-1-x86_64.pkg.tar.zst`, sha256 `e2a0e8a6c65768c9f9b0e47ccf701b4dca03f9f97cda093dc1c0ddf4b23a67c4` | Se extrae `usr/bin/qemu-x86_64-static` con `bsdtar` a `build/tools/`. Sin root. |

- [diseño] `scripts/fetch.py <nombre>` descarga a `build/third_party/<nombre>/`, verifica el sha256 (o el commit) y es idempotente. Los sha256 viven en `scripts/pins.json`, copiados de esta tabla y de `vault/RESEARCH.md`.
- [diseño] Las versiones de la distro no se usan: se compila de fuente fijada para que el binario sea el mismo en cualquier máquina con este compilador.

## 6. Criterios y scripts

Todos los scripts se corren como `python3 scripts/<nombre>.py` desde la raíz del repositorio, llaman a `common.require_non_root()` (falla si `os.getuid() == 0`), nunca usan `sudo`, construyen calco con `cargo build --release` si hace falta y salen con 0 solo si su criterio se cumple. «Igual a nativo» significa mismo código de salida (o misma señal de muerte), stdout byte a byte igual y stderr igual salvo que el script diga otra cosa.

| Criterio de la idea | Script y salida exigida | K |
| --- | --- | --- |
| N secuencias aleatorias, 0 discrepancias | `random_compare.py` (N = 100 000 por defecto, `--n`, `--calco BIN`): imprime `discrepancias: 0`; deja los volcados en `build/random/{native,calco,qemu}/` | K1 |
| Campos del volcado | `dump_fields.py`: valida cada registro de `build/random/` (magia, tamaño, todos los campos de 4.4 presentes y legibles) | K1 |
| Archivo de máscara con referencias | `mask_check.py`: falla si un mnemónico de `isa/scope.tsv` con `flags_undefined` no vacío no está en `harness/undefined_flags.tsv` con `ref` no vacío | K1 |
| Prueba de la máscara | `mask_test.py`: alterar una flag indefinida no da fallo y una definida sí; imprime `mascara-ok` | K1 |
| Mutantes | `mutants.py`: por cada fila de `harness/mutants.tsv`, compila el mutante y corre 20 000 casos; imprime `mutantes detectados: M/M` | K1 |
| Reducción | `reduce_check.py`: reduce un contraejemplo de cada mutante; cada uno sigue fallando y tiene ≤ 3 instrucciones | K1 |
| Cobertura | `coverage.py`: desensambla con `objdump -d -M intel` los ELF generados, imprime los mnemónicos de `isa/scope.tsv` sin cubrir y sale con 0 solo si no hay | K1 |
| qemu como dato | `random_compare.py` también imprime `qemu-x86_64 discrepancias: <n>` (informativo) | K1 |
| No deterministas excluidas | `nondet_check.py`: el generador no emite nada de `harness/exclusions.tsv` y `guests/nondet.c` (RDTSC, RDTSCP, RDRAND, RCPPS, RSQRTPS) corre bajo calco con código 0 | K1 |
| Código del invitado no ejecutable | `noexec.py`: lanza `guests/pause.c` (imprime `ready` y bloquea en `read(0)`), lee `/proc/PID/maps` de calco; imprime `codigo-invitado-no-ejecutable` | K1 |
| CPUID sin AVX, AVX → SIGILL | `cpuid_avx.py`: `guests/avx.c` imprime los bits de CPUID y ejecuta `vaddps`; salida igual a `guests/avx.expected` y muerte por SIGILL | K1 |
| test-x86_64 | `qemu_tcg.py test-x86_64`: compila `test-i386.c` con `-Wall -O0 -g -fno-strict-aliasing -static … -lm -lc` (banderas de QEMU) en `build/qemu-tcg/` y compara con nativo | K1 |
| sha512-sse | `qemu_tcg.py sha512-sse` (`-msse4.1 -O3 -static`) | K1 |
| linux-test | `qemu_tcg.py linux-test` (`-O0 -g -static -lpthread`) | K2 |
| testthread | `qemu_tcg.py testthread`: compara por hilo (la secuencia de líneas de cada prefijo `threadN:` y las líneas restantes) y el multiconjunto de líneas | K2 |
| Señales | `signals.py`: `guests/signals.c` (estático no-PIE) igual a nativo | K2 |
| Hilos | `threads.py`: `guests/threads.c` igual a nativo en R = 50 corridas seguidas | K2 |
| SMC | `smc.py`: `guests/smc.c` igual a nativo | K2 |
| Dinámico con glibc | `dynamic.py`: `guests/hello.c` compilado dinámico (PIE por defecto) igual a nativo y, en pausa, sin `x` en las regiones de ld.so, libc y el ejecutable | K2 |
| busybox | `busybox_run.py` (casos de `targets/busybox/cases.tsv`) y `busybox_suite.py` (suite propia) | K3 |
| SQLite | `sqlite_run.py`: `targets/sqlite/workload.sql`, salida y sha256 de la base iguales | K3 |
| Lua | `lua_run.py`: `targets/lua/*.lua` iguales a nativo | K3 |
| TCC | `tcc_selfcompile.py`: deja `build/tcc/tcc1.native` y `build/tcc/tcc1.calco`, mismo sha256 | K3 |
| Rendimiento, metadatos, RSS | `bench.py`: imprime `calco/qemu: <x>` y `calco/nativo: <y>`; escribe `build/bench/results.json` | K3 |
| Sin root | `common.require_non_root()` en cada script | K1, K3 |
| Arnés menor que traductor | `loc.py`: imprime `traductor: <a>` y `verificacion: <b>`; falla si `b >= a` | K2 (diferido desde K1) |

Reglas de conteo de `loc.py` (las mismas que usa el checker): se cuentan líneas no vacías que no sean solo comentario (`//`, `#`, `/* … */` de una línea). Traductor = `src/**/*.rs` salvo archivos `tests.rs`. Verificación = `harness/**`, `scripts/**`, `guests/**`, `tests/**`, `src/**/tests.rs` (`.py`, `.rs`, `.c`, `.S`, `.s`, `.sh`). Las tablas `.tsv` y las entradas de `targets/` no cuentan.

- [diseño] **`k1.loc` diferido a K2** (2026-10-30, registrado en `CHANGELOG.md`). Al cerrar K1, `python3 scripts/loc.py` dio `traductor: 3927` y `verificacion: 4450`, así que el criterio no se cumple en K1. No se reduce la verificación ni cambian las reglas de conteo de arriba. K2 es el primer K que añade al traductor `execve` (k2-01), señales (k2-02) e hilos (k2-03). Condición de reapertura: verificación < traductor, medido con `scripts/loc.py`, cuando el traductor ya cubra señales, hilos y `execve`.

Casos de señales (`guests/signals.c`): SIGSEGV por escritura a una página sin permiso con un manejador `SA_SIGINFO` que imprime `si_addr`, el `rip` del `ucontext` como desplazamiento respecto a una etiqueta y cuatro registros fijados antes, corrige el contexto (salta la instrucción) y continúa; SIGILL por `ud2`; SIGFPE por división entera entre cero; SIGALRM de `setitimer` en mitad de un bucle de solo registros que mantiene un invariante: el manejador imprime si el `rip` cae dentro del bucle y si el invariante se cumple, no los valores (que dependen del momento).

Busybox: `targets/busybox/cases.tsv` fija al menos 25 applets con entradas en `targets/busybox/inputs/`. Formato: una línea por caso, separada por tabuladores, `stdin<TAB>applet<TAB>arg1<TAB>arg2…`, donde `stdin` es un archivo de `inputs/` o `-` (vacío); las líneas que empiezan con `#` son comentarios. Cada caso corre `busybox applet args…` con `targets/busybox/inputs/` como directorio actual y no escribe archivos: todo sale por stdout. La suite propia (`testsuite/runtest` de busybox) se corre con el binario bajo calco mediante un directorio de enlaces a un envoltorio `exec calco --argv0 "$0" busybox "$@"`; el subconjunto válido es el de las pruebas que pasan en nativo como usuario sin root ni red en dos corridas nativas seguidas; `busybox_suite.py` lo calcula en cada corrida y lo escribe en `build/busybox-suite/suite.list`, con los registros crudos `native.log` y `calco.log` al lado. Se compara el número de `PASS:` dentro del subconjunto.

## 7. Máquina de referencia

- [hecho] CPU: AMD Ryzen 5 5600G (familia 25, modelo 80, stepping 0, microcódigo `0xa500012`), 12 hilos. Tiene SSE–SSE4.2, SSE4A, AVX, AVX2, BMI1/2, ADX, SHA, RDRAND, RDSEED, FSGSBASE, `user_shstk`; no tiene AVX-512.
- [hecho] Linux 7.2.11-arch1-1, glibc 2.44, `ptrace_scope = 1`, `randomize_va_space = 2`, `mmap_min_addr = 65536`, 14 GiB visibles en `free`, usuario con uid 1000.
- [hecho] Herramientas: gcc 16.2.1, clang 23.1.4, rustc/cargo 1.98.1 (con `rustfmt` y `clippy`), Python 3.14.8, GNU Make 4.4.1, binutils 2.47 (`as`, `ld`, `objdump`), gdb 18.1, `taskset` (util-linux 2.42.4), `bsdtar`, `zstd`, `cloc` 2.10, `sqlite3` 3.54.0 y `lua` 5.5.1 de la distro. No están instalados: qemu, busybox, tcc, nasm, perf, valgrind, hyperfine.
- [hecho] qemu-x86_64 11.1.2 del paquete estático de Arch corre sin root extraído con `bsdtar`. `test-x86_64` compilado aquí da salida determinista en nativo (9934 líneas) y 112 líneas distintas bajo qemu-x86_64 (p. ej. `shrdw` con cuenta mayor que 16).
- [hecho] `testthread` de QEMU no es determinista en nativo: en 8 corridas salió 1 orden distinto de líneas.

## 8. Rendimiento

- [diseño] Cargas fijas: (1) `sqlite3` con `targets/sqlite/bench.sql`; (2) `lua` con `targets/lua/bench/*.lua`; (3) TCC compilándose a sí mismo (`tcc_selfcompile` sin comparar); (4) `busybox gzip -9` de un archivo pseudoaleatorio fijo de 16 MiB (`targets/busybox/gen_bench_input.py`, semilla fija) y `busybox sha256sum` del resultado; (5) `sha512-sse` de QEMU.
- [diseño] Metodología: cada carga y cada modo (nativo, qemu-x86_64, calco) fijado al núcleo 2 con `taskset -c 2`; 1 corrida de calentamiento descartada y 5 medidas; tiempo de pared por `time.perf_counter` y `ru_maxrss` por `os.wait4` (dato del kernel). Se toma la mediana por carga y la media geométrica de las razones de medianas. Antes de medir, cada carga se comprueba igual a nativo en los tres modos; si no, el script falla.
- [diseño] Conclusión permitida, declarada antes de medir: calco frente a qemu-x86_64 es **equivalente** (mismo contrato: el mismo binario, salida idéntica, ejecución traducida en modo usuario), así que la razón es una diferencia real de velocidad. calco frente a nativo es **comparable**: es el costo de traducir, nunca un «más rápido».
- [diseño] `results.json` registra `qemu_version` (primera línea de `qemu-x86_64-static --version`), `cpu_model` (`model name` de `/proc/cpuinfo`), `kernel` (`uname -r`), `core` (2) y `workloads`: `{nombre: {"native": [...], "qemu": [...], "calco": [...]}}`, cada lista con las 5 corridas medidas como `{"seconds": float, "maxrss_kb": int}`; además `geomean_calco_qemu` y `geomean_calco_native`. El script falla si falta alguno.

## 9. Umbrales

| Umbral | Valor |
| --- | --- |
| Secuencias aleatorias N | 100 000 (casos con semillas 0…99 999) |
| Casos por mutante | 20 000 |
| Corridas seguidas de hilos R | 50 |
| Tamaño máximo del contraejemplo reducido | 3 instrucciones |
| Límite de RSS de calco por carga | 1 GiB (`ru_maxrss` ≤ 1 048 576 KiB) |
| Media geométrica calco/qemu | ≤ 0.5 |
| Mutantes | M = 6, todos detectados |

## 10. Decisiones cerradas

### D01 — Lenguaje del traductor y del arnés
- Decisión: traductor en Rust estable (edición 2024, solo `std`, `asm!` y `naked_asm!`); arnés y scripts en Python 3 de biblioteca estándar más `as`/`ld`/`objdump` de binutils.
- Razón: Rust da enums y `match` para el decodificador, pruebas integradas y ensamblador en línea estable (comprobado con rustc 1.98.1); Python mantiene el arnés corto, que es un criterio, y binutils hace de ensamblador externo sin escribir otro codificador.

### D02 — Arquitectura del JIT
- Decisión: emisión directa por recodificación de cada instrucción decodificada, sin representación intermedia; solo se reescriben control de flujo, RIP-relativo, segmento GS, `syscall`, `cpuid` y trampas. Optimizaciones: encadenamiento, búsqueda en línea de saltos indirectos y caché cerca del ejecutable; emisor propio en `emit/`.
- Razón: con la misma ISA, recodificar hace que resultado, flags, x87 y SSE salgan del hardware, que es el oráculo; una IR añadiría código y errores sin mejorar ninguna medición del proyecto.

### D03 — Estrategia de flags
- Decisión: flags reales del anfitrión, vivas en RFLAGS entre bloques encadenados; todo código insertado es neutral para flags y solo se guardan con `pushfq` al salir al despachador.
- Razón: elimina el cálculo perezoso al estilo QEMU y hace que incluso las flags indefinidas coincidan con el CPU real; el costo es restringir las expansiones a `mov/lea/jmp/jrcxz`.

### D04 — Asignación de registros
- Decisión: correspondencia 1:1 de los 16 GPR, RFLAGS, XMM y x87; el estado del traductor se alcanza por GS base; los temporales se guardan en `gs:[scratch]`; no hay registros reservados del banco general.
- Razón: misma ISA y mismo número de registros; FSGSBASE está habilitado en esta máquina, así que GS queda libre para el traductor y FS se intercambia barato.

### D05 — Caché de traducciones
- Decisión: arena de 256 MiB con vaciado total al llenarse, tabla hash `rip → bloque`, encadenamiento directo por parcheo de rel32, desenlace por página al invalidar, saltos indirectos y `ret` por despachador en K1 y por sonda en línea con `jrcxz` en K3; sin trazas ni pila de retorno sombra.
- Razón: el vaciado total es la política más simple que no deja punteros colgando; DynamoRIO muestra que caché, enlazado y búsqueda de indirectos dan la mayor parte de la velocidad.

### D06 — Aislamiento del espacio de direcciones
- Decisión: mapeo identidad con aislamiento por colocación (sección 3.1); `MAP_FIXED` que choca con calco devuelve `-ENOMEM`; sin protección contra escrituras salvajes del invitado.
- Razón: el mapeo identidad hace que syscalls y accesos a memoria corran sin traducción de direcciones, que es la base de la velocidad; proteger a calco del invitado costaría una comprobación por acceso.

### D07 — Captura del estado nativo de referencia
- Decisión: ni ptrace ni señales: el ELF de prueba se vuelca a sí mismo con `pushfq` y `fxsave64` y escribe por stdout; el mismo binario se corre en nativo, en calco y en qemu.
- Razón: un solo artefacto para los tres modos, sin depender de `ptrace_scope` ni de que calco ya entregue señales (que llega en K2).

### D08 — Generador, tabla y reducción
- Decisión: generador por plantillas desde `isa/scope.tsv`, memoria acotada a un búfer de 256 bytes, sin fallas por construcción, con seguimiento de flags definidas; reducción ddmin más simplificación de inmediatos.
- Razón: la tabla única define a la vez alcance, cobertura y máscara; ddmin está probado (QuickCheck en TestRIG) y alcanza para un umbral de 3 instrucciones.

### D09 — Máscara o modo estricto
- Decisión: la máscara decide pasa/falla; el modo estricto que iguala al CPU de César se reporta solo como dato.
- Razón: el criterio de la idea es con máscara y, por D03, el modo estricto coincidirá casi siempre; reportarlo mide lo que ganan las flags reales sin convertirlo en un umbral.

### D10 — x87 y MXCSR
- Decisión: x87, MMX y SSE recodificados nativos; estado guardado con `fxsave64`/`fxrstor64` en cada salida al despachador; Rust corre con FCW `0x037F` y MXCSR `0x1F80`; FIP/FDP/FOP fuera de la comparación.
- Razón: precisión, tags y excepciones salen idénticos al hardware sin emularlos; FIP apunta a la caché por construcción.

### D11 — Llamadas al sistema
- Decisión: lista cerrada de la sección 3.10; paso directo por defecto con los mismos argumentos, emulación de memoria, señales, hilos, `execve`, `arch_prctl`, `rseq`/`clone3` con `-ENOSYS`; lo demás `-ENOSYS`.
- Razón: el mapeo identidad vuelve válido el paso directo; la lista cubre lo que usan glibc, busybox, SQLite, Lua, TCC y las pruebas de QEMU.

### D12 — Hilos y señales en mitad de un bloque
- Decisión: un `std::thread` por hilo del invitado, `tid` real; señales síncronas con reconstrucción por tabla `pc → rip`; asíncronas pendientes en Rust o entregadas en la siguiente frontera avanzando con TF; llamadas seguras para los bloqueos.
- Razón: entrega exacta sin poner comprobaciones en el camino rápido de cada bloque.

### D13 — Orden de fases
- Decisión: K1 núcleo y arnés (estáticos, ISA, comparación contra el CPU, pruebas ISA de QEMU); K2 sistema (syscalls completas, señales, hilos, SMC, dinámicos, execve); K3 programas reales y rendimiento.
- Razón: la exactitud de instrucción es la base de todo y se puede medir sin señales ni hilos; los programas reales necesitan todo lo anterior.

### D14 — Umbrales numéricos
- Decisión: N = 100 000, R = 50, contraejemplo ≤ 3 instrucciones, RSS ≤ 1 GiB, calco/qemu ≤ 0.5, 20 000 casos por mutante.
- Razón: 100 000 casos × 3 modos corre en minutos con lotes de 500; los mutantes elegidos se reducen a 1–2 instrucciones; la caché de 256 MiB más el invitado cabe con holgura en 1 GiB; con ejecución nativa de casi todo, ser al menos 2 veces más rápido que qemu es un objetivo exigente pero alcanzable.

### D15 — Cargas y metodología de rendimiento
- Decisión: las cinco cargas de la sección 8, núcleo 2 con `taskset`, 1 calentamiento y 5 medidas, mediana y media geométrica.
- Razón: cubren intérprete (Lua), base de datos con E/S (SQLite), compilador (TCC), enteros y memoria (gzip) y SSE (sha512-sse); fijar núcleo y calentar reduce la varianza sin root.

### D16 — qemu-x86_64 de referencia
- Decisión: `qemu-user-static` 11.1.2-1 de Arch, descargado del espejo con sha256 fijado y extraído con `bsdtar` en `build/tools/`, con respaldo en archive.archlinux.org.
- Razón: estático, sin dependencias y sin root (comprobado en esta máquina); fijar el paquete exacto hace la comparación repetible.

### D17 — CPU de César y `ptrace_scope`
- Decisión: CPU AMD Ryzen 5 5600G (Zen 3) y `ptrace_scope = 1`, observados el 2026-10-29; el decodificador sigue a AMD donde los fabricantes difieren y `bench.py` vuelve a registrar el modelo en cada medición.
- Razón: es el oráculo de todas las comparaciones; ptrace no hace falta por D07.

### D18 — Suites de busybox y SQLite sin root
- Decisión: busybox: el subconjunto de su `testsuite/runtest` que pasa en nativo sin root ni red en dos corridas seguidas, recalculado en cada corrida y escrito en `build/busybox-suite/suite.list`. SQLite: no se usa su suite TCL; se usa una carga SQL fija.
- Razón: el subconjunto se mide en lugar de adivinarse; los criterios de SQLite piden salida y hash de la base, no la suite, y la suite TCL exige construir `testfixture` y tclsh.

### D19 — Versiones de busybox, SQLite, Lua y TCC
- Decisión: busybox 1.36.1, SQLite 3.54.1, Lua 5.5.1 y TCC mob `e23dabed`, compilados de fuente fijada (sección 5); busybox estático y los otros tres dinámicos con la glibc del sistema.
- Razón: versiones actuales verificadas en sus sitios oficiales el 2026-10-29; compilar de fuente da el mismo binario en nativo, qemu y calco, y deja un caso estático y tres dinámicos.

### D20 — glibc y CPUID
- Decisión: confirmado que glibc elige rutinas por el CPUID que ve (sección 3.9); calco filtra CPUID a x86-64-v2 sin XSAVE/OSXSAVE para que glibc no use AVX ni `xsave`.
- Razón: comprobado con `ld.so` bajo `qemu-x86_64 -cpu Nehalem` contra nativo en esta máquina.

### D21 — Compilación de los invitados de prueba
- Decisión: los programas de `guests/` se compilan con `gcc -O2 -static` (no-PIE) salvo `dynamic.py`, que usa el PIE dinámico por defecto de gcc.
- Razón: direcciones fijas permiten imprimir RIP exactos en la prueba de señales y empezar K1 sin intérprete dinámico.

### D22 — testthread no determinista
- Decisión: `testthread` se compara por secuencia de líneas de cada hilo y por multiconjunto, no por orden global.
- Razón: en nativo el orden global cambia entre corridas (observado); comparar el orden global haría fallar a nativo contra sí mismo.

### D23 — Dónde se guardan los mutantes
- Decisión: los mutantes son *features* de Cargo dentro del traductor, apagadas por defecto.
- Razón: el criterio pide fallos inyectados «en el traductor»; una feature no cambia el binario normal y cuesta una línea por mutante.

### D24 — Licencia
- Decisión: MIT, a nombre de César Alberto Manzo Olivares.
- Razón: es un proyecto de portafolio pensado para ser público; no incorpora código de QEMU (sus pruebas se descargan a `build/`, no se versionan).

### D25 — Comando único de chequeo
- Decisión: `make check` corre `cargo fmt --check` y `cargo test --release` si existe `Cargo.toml`, y `python3 -m unittest discover -s harness -p 'test_*.py'` si hay pruebas del arnés. Los scripts largos de la sección 6 no van en `make check`: los corren los checkers al cierre de cada K.
- Razón: el método pide formato, tipos y pruebas en el chequeo de trabajo; los scripts de comparación tardan minutos y necesitan descargas.

## 11. Contradicciones resueltas

- **Tamaño del verificador.** method.md da como referencia que la verificación ronde el 15 % del producto; la idea exige solo que el arnés sea menor que el traductor y además pide un arnés con generador, reductor y mutantes. Gana la idea en lo que toca al producto: el criterio que pasa/falla es `verificacion < traductor` (sección 6). El 15 % se reporta como dato al cerrar cada K.
- **Mutantes.** method.md deja fuera los mutantes como puerta de cierre; la idea los pide como criterio. Gana la idea: son un criterio de producto (el arnés detecta fallos), no una puerta de cada mini sprint.
- **«Igual a nativo» en testthread.** La idea pide la misma salida; en nativo el orden no es estable. Se resuelve con D22.
