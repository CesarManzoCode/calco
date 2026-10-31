# El JIT de calco

Este documento explica cómo traduce calco el código de un programa x86-64 para un lector que no conoce el proyecto. Describe el diseño fijado en la sección 3 de [`CONTRACT.md`](CONTRACT.md) y dónde vive cada parte en `src/`. Lo que calco no hace o no demuestra está solo en [`LIMITATIONS.md`](../LIMITATIONS.md); aquí no se repite.

## Idea general

calco es un traductor binario dinámico de x86-64 a x86-64. Como el invitado y el anfitrión tienen la misma ISA, calco no interpreta ni pasa por una representación intermedia: decodifica cada instrucción del invitado, la **vuelve a codificar** en una caché de código propia y ejecuta esa copia. El hardware calcula los resultados, las flags (también las indefinidas), las excepciones y el estado x87/SSE. calco solo reescribe lo que no puede correr tal cual: el control de flujo, los operandos relativos a RIP, el segmento GS, `syscall`, `cpuid` y las trampas.

Tres decisiones sostienen el resto:

- **Mapeo identidad** (`src/mem.rs`): una dirección del invitado es la misma dirección del anfitrión. Los punteros que el invitado pasa al kernel valen tal cual y los accesos a memoria no se traducen.
- **Registros 1:1** (`src/cpu.rs`): mientras corre código traducido, los 16 registros generales, RFLAGS, los XMM, x87/MMX y MXCSR del invitado viven en los mismos registros del anfitrión, y RSP del invitado es RSP del anfitrión. El estado del traductor se alcanza por la base de GS, que apunta al `struct Cpu` del hilo.
- **Ninguna página del invitado es ejecutable** en el anfitrión: el código del invitado solo se lee para traducirlo.

El ciclo de un hilo está en `run` (`src/jit/mod.rs`): busca el bloque del `rip` actual en la caché (o lo traduce), entra con el trampolín, y cuando el bloque sale atiende el motivo (encadenar, llamada al sistema, `cpuid`, señal) y vuelve a empezar.

## Recorrido de un bloque

Un ejemplo ilustrativo (no es una salida volcada de calco) muestra cómo se combinan las reglas de las secciones siguientes. Sea este código de un `ET_EXEC` en `0x401000`:

```
401000: mov  eax, [rip+0x2000]
401006: add  eax, 1
401009: cmp  eax, ebx
40100b: jne  401020
40100d: ...
```

1. El hilo llega al despachador con `rip = 0x401000`. La búsqueda sin candado en `BlockMap` falla, así que `locked_block` toma el candado de la caché, traduce desde `0x401000` y publica el bloque.
2. `mov eax, [rip+0x2000]` es `Plain` con operando RIP-relativo: se recodifica con un desplazamiento nuevo que apunta a la misma dirección del invitado (`0x403006`), porque la caché está a menos de 2 GiB del ejecutable.
3. `add` y `cmp` son `Plain` sin nada que reescribir: salen con los mismos prefijos, opcode y operandos. Las flags que deja `cmp` son las del hardware.
4. `jne` termina el bloque. Se emite `jne rel32` hacia el stub de la salida 0 (destino `0x401020`) y `jmp rel32` hacia el stub de la salida 1 (destino `0x40100d`). Antes de cada salto se rellena con `nop` para que su campo rel32 quede alineado a 4 bytes y se pueda parchear con una sola escritura.
5. La tabla de pc del bloque dice, para cada tramo de código del anfitrión, en qué instrucción del invitado está el estado (`0x401000`, `0x401006`, …) y que ningún registro vive en `scratch`.
6. El trampolín `enter` carga los registros del invitado y salta al bloque. Si `jne` se toma, el stub de la salida 0 escribe `0x401020` en `gs:[next_rip]` y `EXIT_BRANCH` con el número de salida en `gs:[exit_reason]`, y salta al *thunk* de salida.
7. De vuelta en `run`, `rip` pasa a `0x401020` y el despachador recuerda la salida 0. En la vuelta siguiente traduce `0x401020` bajo el candado y parchea el rel32 del `jne` para que apunte al bloque nuevo. Desde entonces ese camino ya no sale a Rust.

## Motivos de salida

El código traducido solo vuelve a Rust por el trampolín de salida, con el motivo en la palabra baja de `Cpu.exit_reason` (constantes `EXIT_*` de `src/jit/translate.rs`). `run` (`src/jit/mod.rs`) lo atiende así:

| Motivo | Quién lo produce | Qué hace el despachador |
| --- | --- | --- |
| `EXIT_BRANCH` (1), con el número de salida en la palabra alta | El stub de una salida directa todavía sin encadenar | Traduce el destino si falta y encadena esa salida si es de la generación actual de la caché |
| `EXIT_INDIRECT` (2) | La salida compartida, cuando la sonda de un salto indirecto falla | Busca o traduce el bloque de `next_rip` y llena su casilla en `LookupTable` |
| `EXIT_SYSCALL` (3) | `syscall` del invitado | `syscall::handle`; `exit` termina el hilo |
| `EXIT_CPUID` (4) | `cpuid` del invitado | `cpuid::handle` con las hojas filtradas |
| `EXIT_SIGNAL` (5), con la señal en los bits 8–15 | `ud2`, `int3`, `int n`, `hlt`, privilegiadas, opcodes fuera del alcance | `signal::trap` entrega al invitado la señal que daría el kernel |
| `EXIT_INTERRUPTED` (6) | El manejador de señales del anfitrión o `enter` con una señal pendiente | Nada: la vuelta siguiente del ciclo entrega lo pendiente |

Antes de cada entrada el despachador atiende, en este orden, una escritura de código automodificable diferida, las señales pendientes que se pueden entregar y la búsqueda del bloque. La búsqueda sin candado solo vale si no hay que encadenar, ni llenar una casilla, ni ejecutar una sola instrucción; en esos casos toma el candado.

## Decodificación

El decodificador está en `src/decode/mod.rs` y sus tablas en `src/decode/table.rs`. Es *table-driven*: hay una tabla por mapa de opcodes (un byte, `0F`, `0F 38` y `0F 3A`) y cada casilla dice qué clase de instrucción es y qué operandos lleva. Primero se consumen los prefijos de los grupos 1 a 4, REX y los prefijos obligatorios `66/F2/F3`; luego el opcode, ModRM, SIB, desplazamiento e inmediato.

El resultado es una `Insn` con todos los campos que el emisor necesita para recodificarla: prefijos tal cual, REX, mapa, opcode, ModRM/SIB/desplazamiento, inmediato, longitud, si usa RIP-relativo o segmento FS/GS, y una clase (`Plain`, `Branch`, `Call`, `Ret`, `IndirectBranch`, `Syscall`, `Cpuid`, `Trap`, `Invalid`). La clase decide cómo se traduce.

El conjunto aceptado es exactamente la tabla del alcance, `isa/scope.tsv`, más unas pocas codificaciones heredadas que el hardware trata como otra instrucción del mismo opcode. Lo que no está en la tabla, incluidos VEX y EVEX, es `Invalid` y el invitado recibe SIGILL con el `rip` en esa instrucción. Donde Intel y AMD decodifican distinto se sigue al CPU de referencia (AMD Zen 3). El decodificador nunca ejecuta lo que lee; solo lee bytes de memoria con permiso lógico de ejecución, a través del rasgo `GuestCode` de `src/jit/translate.rs`.

`src/cpuid.rs` completa el cuadro: el CPUID que ve el invitado es el del anfitrión filtrado a x86-64-v2 sin XSAVE, así que glibc nunca elige rutinas con instrucciones fuera de la tabla.

## Emisión

`src/emit/mod.rs` es un codificador de instrucciones x86-64 del anfitrión, sin dependencias. Su operación central, `Asm::recode`, vuelve a codificar una `Insn` a partir de sus campos —nunca copia los bytes del invitado— con los mismos prefijos, REX, opcode, operandos e inmediato, y opcionalmente con un operando de memoria nuevo. Los demás métodos emiten las pocas formas que necesitan el traductor y los trampolines.

`src/jit/translate.rs` arma un **bloque**: decodifica desde un `rip` y emite instrucción por instrucción hasta la primera de control de flujo, `syscall`, `cpuid` o trampa, al cruzar 4 KiB de código del invitado o a las 64 instrucciones (`MAX_INSNS`). Las reescrituras son:

- **RIP-relativo**: si el destino queda a menos de ±2 GiB del código traducido, la instrucción se recodifica con un desplazamiento nuevo; si no, se carga la dirección en un temporal con `mov r, imm64` y el operando pasa a ser `[r]`. La caché se coloca cerca del ejecutable (sección «Memoria») para que el primer caso sea el habitual.
- **Segmento GS del invitado**: la dirección efectiva se calcula en un temporal con `lea` y se le suma el `gs_base` del invitado guardado en `Cpu`. El segmento FS va nativo, porque la base de FS del anfitrión es la del invitado mientras corre código traducido.
- **Control de flujo**: saltos, llamadas y retornos (secciones «Caché y encadenamiento» y «Saltos indirectos»).
- **`syscall` y `cpuid`**: salida al despachador, que los atiende en `src/syscall/` y `src/cpuid.rs`. **`ud2`, `int3`, `int n`, `hlt` y privilegiadas**: salida con la señal que daría el kernel.

Cuando una expansión necesita un registro, `pick_temp` elige uno que la instrucción no nombre, lo guarda en `gs:[scratch]` y lo restaura después. Nada se escribe en la pila del invitado salvo la dirección de retorno de un `call`, que la instrucción original también escribe; así se respeta la zona roja de 128 bytes. Los helpers en Rust corren en la pila del anfitrión.

Cada bloque guarda una **tabla de pc** (`PcEntry`): para cada tramo de código del anfitrión, en qué frontera de instrucción del invitado está el estado y qué registros viven en ese momento en `scratch`. Con ella se reconstruye el estado exacto del invitado desde cualquier byte del bloque; la usan las señales y el código automodificable.

## Flags

calco no calcula flags perezosas. Como las instrucciones se recodifican, cada una deja en RFLAGS exactamente lo que dejaría el CPU real, incluidas las flags que el manual deja indefinidas. Para que eso valga entre instrucciones y entre bloques, **todo el código que inserta el traductor es neutral para flags**: solo usa `mov`, `movzx`, `lea`, `jmp`, `jrcxz`, `nop` y `pop gs:[next_rip]`, más los `jcc`/`loop`/`jrcxz` del propio invitado. Esa restricción está escrita como invariante en la cabecera de `src/jit/translate.rs`.

Las flags se guardan en `Cpu.rflags` solo al salir al despachador: el trampolín de salida (`exit_trampoline` en `src/jit/trampoline.rs`) hace `pushfq` en la pila del anfitrión y limpia DF antes de entrar a Rust; el de entrada (`enter`) las restaura con `popfq`. Entre bloques encadenados las flags siguen vivas en el registro.

El mismo par de trampolines guarda y restaura el estado x87/SSE con `fxsave64`/`fxrstor64`, de modo que Rust siempre corre con FCW `0x037F` y MXCSR `0x1F80`, y el invitado recupera su precisión, sus tags y sus excepciones tal como las dejó.

## Caché y encadenamiento

La caché de código (`Jit` en `src/jit/mod.rs`) es una arena de 256 MiB escrita y ejecutada por calco. Empieza con una zona fija —el *thunk* de salida hacia el trampolín y la salida compartida de los saltos indirectos— y después vienen los bloques, alineados a 16 bytes. Una tabla hash de direccionamiento abierto y tamaño fijo, `BlockMap`, va de `rip` del invitado a bloque. Cuando la arena o la lista de bloques se llenan, la caché se **vacía completa** (`flush`) y se vuelve a empezar; no hay expulsión parcial, así nunca quedan punteros colgando hacia un bloque borrado.

Cada salida directa de un bloque es un `jmp rel32` (o un `jcc rel32` seguido de un `jmp rel32` en los saltos condicionales) que al principio apunta a un *stub* propio. El stub escribe el `rip` de destino en `gs:[next_rip]` y el motivo `EXIT_BRANCH` con el número de la salida, y salta al trampolín de salida. El despachador traduce el destino si hace falta y **parchea** el rel32 para que apunte directamente al bloque nuevo (`link`, `patch_rel32`). El parche es una escritura atómica de 4 bytes alineada, así que un hilo que ejecuta ese código ve el salto viejo o el nuevo, nunca una mezcla. A partir de ahí los dos bloques quedan **encadenados** y el camino ya no pasa por Rust.

Cada bloque recuerda qué salidas de otros bloques apuntan a él. Cuando un bloque se invalida (sección «Código automodificable»), sus enlaces entrantes y salientes vuelven a sus stubs, su `rip` sale de la tabla dejando una lápida y su código queda en la arena hasta el siguiente vaciado: un hilo que todavía esté dentro lo termina, y el manejador de señales todavía puede leer su tabla de pc.

Las llamadas directas escriben la dirección de retorno del invitado en su pila (`mov [rsp-8], imm32`, o con un temporal si no cabe en 32 bits, y luego `lea rsp,[rsp-8]`) y siguen como un `jmp` encadenable. No hay pila de retorno sombra.

## Saltos indirectos

Un salto o una llamada indirecta y un `ret` no conocen su destino al traducir. El código traducido lleva el destino del invitado —de un registro, de memoria o, en `ret`, con `pop gs:[next_rip]` desde la pila del invitado— a `gs:[next_rip]`, y después hace una **búsqueda en línea** (`indirect` en `src/jit/translate.rs`) en una tabla de correspondencia directa `rip → código` (`LookupTable` en `src/jit/mod.rs`).

La tabla tiene 65 536 casillas indexadas por los 16 bits bajos del `rip`. Cada casilla es una palabra de 64 bits: la mitad baja guarda el negativo de los bits 16–47 del `rip` dueño y la mitad alta el desplazamiento de su código dentro de la caché. La sonda:

1. guarda RCX en `scratch` y comprueba con `movzx` + `jrcxz` que los bits 48–63 del destino son cero (si no, sale al despachador);
2. guarda RAX, lee la casilla de los 16 bits bajos y suma con `lea` los bits 16–47 del destino a la mitad baja de la entrada: el resultado es cero solo si coinciden, y `jrcxz` lo comprueba;
3. si coinciden, calcula la dirección de destino (base de la caché más la mitad alta) en `scratch`, restaura RCX y RAX y salta con `jmp gs:[scratch]`.

Todo es `mov`, `movzx`, `lea`, `jrcxz` y `jmp`, así que las flags del invitado no se tocan y la pila del invitado tampoco. Si la sonda falla, sale por la salida compartida con `EXIT_INDIRECT`; el despachador encuentra o traduce el bloque y llena la casilla bajo el candado de la caché. Cada casilla se publica con una sola escritura atómica, por eso los hilos la leen sin candado. Una casilla libre apunta a la salida compartida, de modo que «coincidir» con ella también lleva al despachador. Invalidar un bloque vacía su casilla y vaciar la caché las vacía todas.

## Memoria

`src/mem.rs` lleva el mapa de memoria del invitado: los intervalos que le pertenecen, con los permisos **lógicos** que pidió. En el anfitrión, toda página del invitado se mapea sin `PROT_EXEC` (`host_prot`); que una página sea «ejecutable» es solo un dato del mapa, que el traductor consulta antes de leer código. Lo que el anfitrión tiene mapeado y no está en ese mapa es de calco.

El aislamiento entre invitado y traductor es por **colocación**, no por protección. calco ocupa lo que el kernel le da en direcciones altas, y el invitado vive en direcciones bajas: un `ET_EXEC` en sus `p_vaddr`, un `ET_DYN` a partir de `0x0000_1000_0000_0000`, el `brk` tras el segmento más alto del ejecutable, la caché de código 1 GiB más arriba alineada a 2 MiB (para que el código traducido quede a menos de 2 GiB del ejecutable y los operandos RIP-relativos se recodifiquen directo), la ventana de colocación de bibliotecas y `mmap` después de la caché, y la pila principal con tope en `0x0000_4000_0000_0000`. Un `mmap` fijo que chocaría con memoria de calco se rechaza.

`src/loader.rs` carga el ELF (y su intérprete `ld-linux-x86-64.so.2` si lo pide) solo como datos, arma la pila inicial con el auxv del ABI System V y no entrega vDSO. Las llamadas de memoria del invitado (`brk`, `mmap`, `munmap`, `mprotect`, `mremap`, `shmat`, `shmdt`) se emulan en `src/syscall/mod.rs` sobre `mem.rs`, y las que quitan o cambian código ya traducido invalidan sus bloques.

## Señales

Las señales están en `src/signal/`: `mod.rs` lleva las acciones del invitado, las máscaras, las pendientes, el marco y `rt_sigreturn`; `host.rs`, el manejador del anfitrión y las llamadas seguras; `frame.rs`, el `rt_sigframe` de x86-64 con los desplazamientos de los encabezados del kernel.

calco instala un manejador propio para cada señal que el invitado atrapa y siempre para SIGSEGV, SIGBUS, SIGILL, SIGFPE y SIGTRAP. Corre en una pila alternativa por hilo, así que no toca la pila del invitado ni su zona roja, y su entrada en ensamblador restaura la base de FS de calco antes de entrar a Rust. El manejador **nunca construye el marco del invitado**: lleva el hilo de vuelta al despachador en una frontera de instrucción del invitado con el estado exacto en `Cpu`, y es el despachador el que llama a `deliver`.

- **Síncronas** (falla de memoria, `ud2`, división entre cero, excepciones x87/SSE, `int3`): el `pc` del `ucontext` está dentro de un bloque. Con `block_at` se encuentra el bloque y con su tabla de pc el `rip` del invitado y los temporales en `scratch`, que se restauran. Como cada instrucción recodificada falla antes de tener efecto, ese es el estado exacto del invitado.
- **Asíncronas**: si llegan mientras corre Rust, quedan en `Cpu.pending_signals` y el despachador las entrega antes de entrar al siguiente bloque. Si llegan en código traducido justo en una frontera, el hilo vuelve al despachador de inmediato. Si caen en medio de una expansión, el manejador enciende TF en el `ucontext` y avanza instrucción por instrucción del anfitrión hasta la siguiente frontera. Al arrancar, calco prueba si el kernel respeta TF desde `rt_sigreturn`; si no, desenlaza el bloque y la señal se entrega cuando el bloque sale.
- **Llamadas seguras**: las llamadas al sistema que pueden bloquear salen desde un stub cuyo rango conoce el manejador. Si una señal llega entre la comprobación de pendientes y la instrucción `syscall`, el manejador salta la llamada, y después se reinicia o termina con `EINTR` según `SA_RESTART` del invitado.

Los números de señal se traducen: las 32 y 33 del invitado (internas de glibc) son las 64 y 63 del anfitrión, y la 62 del anfitrión es de calco. El marco del invitado tiene el formato del kernel con el área de punto flotante en formato FXSAVE. Cuando la acción por defecto mata, calco restablece la acción del anfitrión y se reenvía la señal, así el proceso muere igual que en nativo.

## Hilos

Un hilo del invitado es un `std::thread` del anfitrión con su propio `Cpu`, su base de GS y su pila alternativa de señales (`src/thread.rs`). `clone` con `CLONE_VM|CLONE_THREAD` arranca el hijo con los registros del padre en el `syscall`, `rax = 0`, la pila pedida y, con `CLONE_SETTLS`, el `fs_base` pedido. Su `tid` es el `gettid` real, así que `futex` pasa directo al kernel. calco escribe `CLONE_PARENT_SETTID`/`CLONE_CHILD_SETTID` antes de que el padre siga y emula `CLONE_CHILD_CLEARTID` al terminar el hilo.

Todos los hilos comparten la caché. Traducir, parchear e invalidar se hace bajo `Mutex<Jit>`; el mapa de memoria y el estado del proceso, bajo `Mutex<World>`; el orden de candados es caché, mundo, lista de hilos. Lo que los hilos leen sin candado —la tabla de bloques, la lista de bloques y la tabla de saltos indirectos— está agrupado en `Index` y se publica con escrituras atómicas: el bloque antes que su `rip`, el `rip` con *release*, y el lector lo ve con *acquire*.

Vaciar la caché exige un **punto seguro**: un hilo está «en la caché» desde antes de buscar un bloque hasta que vuelve a Rust desde `enter`. El hilo que vacía levanta `FLUSHING`, manda la señal 62 del anfitrión a cada hilo que siga dentro (su manejador lo lleva al despachador en una frontera de instrucción), espera a que no quede ninguno, vacía y despierta a los que esperan en la barrera.

## Procesos y `execve`

`fork` y `vfork` se emulan como `fork`: el hijo es una copia del proceso entero, calco incluido, y sigue traducido. `execve` (`src/syscall/mod.rs`) comprueba primero el archivo con `loader::resolve`, así el invitado recibe el mismo errno que daría el kernel (`ENOENT`, `EACCES`, `ENOEXEC`) y sigue corriendo. Si el archivo es ejecutable, calco se vuelve a ejecutar a sí mismo —la ruta de `/proc/self/exe` que leyó al arrancar— con `--argv0 <argv[0]> -- <ruta> <args…>` y el entorno del invitado, y el programa nuevo también corre traducido. Una ruta `/proc/self/exe` del invitado significa su propio programa, no calco.

Si lo que el invitado ejecuta es el propio binario de calco (mismo dispositivo e inodo, siguiendo enlaces), calco lo ejecuta tal cual, sin envolverlo: es lo que pasa cuando un programa traducido ejecuta un envoltorio `exec calco … PROGRAMA`, como el de la suite de busybox. Un calco traducido por otro calco pondría la pila y la caché del invitado interno en las mismas direcciones que las del externo, y el calco interno ya traduce `PROGRAMA`.

El kernel nombra el proceso (`/proc/PID/comm`) por el archivo que ejecutó, que es calco. Al arrancar, calco cambia ese nombre con `prctl(PR_SET_NAME)` por el último componente del `argv[0]` del invitado, que es lo que buscan `ps`, `pidof` y `killall`.

## Código automodificable

`src/smc.rs` lleva la cuenta de qué páginas del invitado tienen código traducido: cada bloque se anota en las páginas que tocan sus bytes (una, o dos si la última instrucción cruza). La primera vez que se traduce desde una página que el invitado puede escribir, calco le **quita la escritura en el anfitrión**.

Una escritura del invitado en esa página falla con SIGSEGV en calco. Si el candado de la caché está libre, el manejador invalida todos los bloques de la página, le devuelve la escritura y retoma en la frontera reconstruida con la tabla de pc: la escritura se repite en su sitio o, si su propio bloque fue invalidado, el despachador la ejecuta sola en un bloque de una instrucción (`translate_step`) y el código que sigue se traduce de nuevo desde lo que escribió. Si el candado está ocupado, el hilo vuelve al despachador con la falla anotada en `Cpu.smc_write` y el despachador decide con el candado (`deferred_write`). El invitado no recibe nada en ningún caso.

Lo mismo se aplica, sin falla, a los rangos que tocan `munmap`, `mprotect` sin ejecución, `mremap`, un `mmap` fijo o `madvise(MADV_DONTNEED)` (`invalidate_range`). Un hilo que está dentro de un bloque cuando otro lo invalida lo termina, la misma garantía que da el hardware sin una instrucción serializante.

## Arnés

El arnés (`harness/`, sección 4 del contrato) usa el CPU como oráculo. `harness/gen.py` genera, con semillas fijas, casos de 1 a 16 instrucciones a partir de `isa/scope.tsv`, con memoria acotada a un búfer de 256 bytes, sin fallas por construcción y con un seguimiento de qué flags quedan definidas. `harness/build.py` los ensambla con `as`/`ld` en un ELF estático sin libc que carga el estado inicial, ejecuta cada caso y se vuelca a sí mismo (`pushfq`, registros, `fxsave64`, memoria) por stdout. Ese mismo binario corre en nativo, bajo calco y bajo `qemu-x86_64` (`harness/run.py`), y `harness/compare.py` compara los volcados campo por campo con la máscara de flags indefinidas. `harness/reduce.py` reduce un contraejemplo con ddmin.

Que el arnés detecta fallos se comprueba con seis mutantes: *features* de Cargo de una línea en `src/emit/` o `src/jit/` que estropean una instrucción concreta. Los scripts de `scripts/` cubren el resto de la sección 6 del contrato: las pruebas de QEMU, señales, hilos, SMC, invitados dinámicos, los programas objetivo (busybox, SQLite, Lua, TCC) y `scripts/bench.py`, que mide el rendimiento contra nativo y contra `qemu-x86_64` y deja `build/bench/results.json`.

## Mapa del código

| Archivo | Qué contiene |
| --- | --- |
| `src/main.rs` | CLI, nombre del proceso, arranque del hilo principal |
| `src/loader.rs` | ELF, intérprete, `#!`, pila inicial y auxv |
| `src/mem.rs` | Intervalos del invitado, permisos lógicos, colocación, `brk` |
| `src/cpu.rs` | `struct Cpu` y sus desplazamientos fijos |
| `src/decode/` | Decodificador y tablas del alcance |
| `src/emit/` | Codificador x86-64 del anfitrión y `recode` |
| `src/jit/translate.rs` | Traducción de un bloque, sonda de saltos indirectos, tabla de pc |
| `src/jit/mod.rs` | Arena, `BlockMap`, `LookupTable`, encadenamiento, invalidación, despachador |
| `src/jit/trampoline.rs` | Trampolines de entrada y salida |
| `src/syscall/` | Lista cerrada de llamadas al sistema, emulaciones, `execve` |
| `src/signal/` | Acciones, manejador del anfitrión, marcos, llamadas seguras |
| `src/thread.rs` | `clone`, lista de hilos, punto seguro |
| `src/smc.rs` | Páginas con traducciones y su protección |
