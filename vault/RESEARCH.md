# Investigación de calco

Fuentes primarias con su revisión fijada. Lo marcado **[medido]** se ejecutó en la máquina de César el 2026-10-29; lo marcado **[consultado]** se leyó en la fuente oficial ese mismo día. La investigación previa a K0 (artículos, comparables, flags indefinidas por fabricante) se resume al final con sus enlaces.

## Herramientas y versiones

| Herramienta | Revisión fijada | Cómo se verificó |
| --- | --- | --- |
| rustc / cargo | 1.98.1 (48a229cea 2026-09-01) / 1.98.1 (797e8a9bc 2026-08-05) | [medido] `rustc --version`, `cargo --version`; compilado un programa con `#[unsafe(naked)]`, `naked_asm!` y `asm!("rdgsbase …")`, edición 2024 |
| rustfmt / clippy | 1.9.0-stable / 0.1.98 | [medido] `cargo fmt --version`, `cargo clippy --version` |
| gcc | 16.2.1 20260810 | [medido] `gcc --version`; `gcc -static` produce `ET_EXEC` estático (`file`) |
| binutils (`as`, `ld`, `objdump`) | 2.47 | [medido] `as --version`, `ld --version`, `objdump --version` |
| Python | 3.14.8 | [medido] `python3 --version`; `unittest` y `tomllib` importan |
| GNU Make | 4.4.1 | [medido] `make --version` |
| glibc | 2.44 | [medido] `ldd --version`; `/usr/lib/libc.a`, `libm.a`, `libpthread.a` presentes (enlace estático posible) |
| Linux | 7.2.11-arch1-1 | [medido] `uname -r`; `AT_HWCAP2 = 0x2` (FSGSBASE) con `LD_SHOW_AUXV=1 /bin/true` |
| qemu-x86_64 | 11.1.2 (Arch `qemu-user-static` 11.1.2-1) | [medido] descargado de `https://geo.mirror.pkgbuild.com/extra/os/x86_64/qemu-user-static-11.1.2-1-x86_64.pkg.tar.zst`, sha256 `e2a0e8a6c65768c9f9b0e47ccf701b4dca03f9f97cda093dc1c0ddf4b23a67c4`; `bsdtar -xf … usr/bin/qemu-x86_64-static`; `--version` → `qemu-x86_64 version 11.1.2`; ELF static-pie. Respaldo en `https://archive.archlinux.org/packages/q/qemu-user-static/qemu-user-static-11.1.2-1-x86_64.pkg.tar.zst` (HTTP 200) |
| pacman (solo lectura) | — | [medido] `pacman -Si`: `qemu-user-static 11.1.2-1` sin dependencias; `qemu-user` depende de glib2, capstone y otras; `busybox 1.36.1-4`, `tcc 0.9.27.r1337.gb39cbc70-1`, `lua 5.5.1-1` |
| GitHub Actions `actions/checkout` | v7 | [consultado] `git ls-remote --tags https://github.com/actions/checkout` (último mayor `v7`) |
| taskset | util-linux 2.42.4 | [medido] `taskset --version` |
| bsdtar, zstd | presentes | [medido] `command -v` |

## Programas objetivo y pruebas externas

| Fuente | Revisión fijada | sha256 / verificación |
| --- | --- | --- |
| busybox | 1.36.1, `https://busybox.net/downloads/busybox-1.36.1.tar.bz2` | `b8cc24c9574d809e7279c3be349795c5d5ceb6fdf19ca709f80cde50e47de314` [medido] |
| SQLite | 3.54.1, `https://www.sqlite.org/2026/sqlite-amalgamation-3540100.zip` (listado en `https://www.sqlite.org/download.html`) | `0a9c8e34352dad46006aa82836d5383bff9dc6bc9ce5a60553c05aa30e6ad1f9` [medido] |
| Lua | 5.5.1, `https://www.lua.org/ftp/lua-5.5.1.tar.gz` (último de `https://www.lua.org/ftp/`) | `1c4b4068d67061f2a2231ad2b5422e77acea1487ea9890f6320af614f4373dce` [medido] |
| TCC | `https://repo.or.cz/tinycc.git`, rama `mob` en `e23dabed154a8495f3ac1ff3cc9d04fa1c207893` | [consultado] `git ls-remote`; `release_0_9_27` = `d348a9a51d32cece842b7885d27a411436d7887b` (2017, no se usa) |
| QEMU tests/tcg | tag `v11.1.2` → commit `4fc49f46dc95d4a27de2509e7fceb2931e91faeb` (`git ls-remote https://gitlab.com/qemu-project/qemu.git`) | archivos de `https://gitlab.com/qemu-project/qemu/-/raw/<commit>/tests/tcg/…` [medido]: |
| ↳ `i386/test-i386.c` | | `f43060b677a63ce403e3c56229877a944abb69f76d87d78e162bfb9ad0e6287e` |
| ↳ `i386/test-i386.h` | | `f7a282f03e66f703c1e67ce38d9032607e062c973c318bc119de2b7a109bab47` |
| ↳ `i386/test-i386-shift.h` | | `889acabbf3aaae96bf552796280d1ca006d75f5ea72835b8d852c27d411f1eed` |
| ↳ `i386/test-i386-muldiv.h` | | `2ae14009409e192eeca56af37da23be4f6134964b417bdf2eec1989446ac694f` |
| ↳ `multiarch/linux/linux-test.c` | | `a558f9abd17eaae157bb0f610039d429e6452eeac7af8560d0f6eb4a034e22a0` |
| ↳ `multiarch/testthread.c` | | `c7b0c7f0b33d42240f69d6b9a4ddd65e42133e1045659e5cde7a59419da7662c` |
| ↳ `multiarch/sha512.c` | | `675829f3b5bebb78c19cd687319f52a7ef3a881ccb618b24f40bbe379a9f86ab` |

Banderas de compilación de QEMU [consultado] en ese commit: `tests/tcg/Makefile.target` pone `-Wall -Werror -O0 -g -fno-strict-aliasing` y `LDFLAGS=-static`; `x86_64/Makefile.target` construye `test-x86_64` desde `test-i386.c` con `-lm -lc`; `i386/Makefile.target` construye `sha512-sse` con `-msse4.1 -O3`; `multiarch/Makefile.target` enlaza `testthread` con `-lpthread`. `-Werror` se omite en calco porque gcc 16 puede añadir avisos que la versión de QEMU no previó; el binario es el mismo para nativo, qemu y calco.

## Observaciones en la máquina

- [medido] `/proc/cpuinfo`: AMD Ryzen 5 5600G, familia 25, modelo 80, stepping 0, microcódigo `0xa500012`; flags incluyen `sse … sse4_2 sse4a avx avx2 bmi1 bmi2 adx sha_ni rdrand rdseed fsgsbase user_shstk`; sin `avx512f`.
- [medido] `/proc/sys/kernel/yama/ptrace_scope` = 1; `randomize_va_space` = 2; `mmap_min_addr` = 65536.
- [medido] glibc respeta CPUID: nativo, `ld.so --help` lista `x86-64-v3 (supported, searched)` y `--list-diagnostics` da `x86.cpu_features.preferred.AVX_Fast_Unaligned_Load=0x1`; bajo `qemu-x86_64-static -cpu Nehalem` solo `x86-64-v2 (supported, searched)` y `=0x0`.
- [medido] `test-x86_64` (gcc 16, `-O2 -fno-pie -no-pie`, `-lm -lc`): 9934 líneas, igual en dos corridas nativas; bajo qemu-x86_64 11.1.2, 112 líneas distintas (la primera: `shrdw` con `C=0x13`).
- [medido] `linux-test`, `testthread`, `sha512` y `sha512-sse` estáticos: código 0 en nativo y bajo qemu; `testthread` dio 2 órdenes distintos en 8 corridas nativas.

## Manuales de referencia

- AMD64 Architecture Programmer's Manual, Vol. 3 (publicación 24594; `https://docs.amd.com/v/u/en-US/24594_3.37` respondió HTTP 200 el 2026-10-29), referencia principal para flags indefinidas porque el oráculo es un AMD. El sprint que transcribe `harness/undefined_flags.tsv` escribe en su cabecera la revisión exacta que consultó en `https://docs.amd.com`.
- Intel 64 and IA-32 Architectures Software Developer's Manual, Vol. 2, referencia secundaria (en `https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html`), misma regla de cabecera.
- Formatos del kernel tomados de los encabezados instalados (Linux 7.2.11 / glibc 2.44): `/usr/include/asm/unistd_64.h`, `/usr/include/asm/sigcontext.h`, `/usr/include/asm/ucontext.h`, `/usr/include/asm/signal.h`, `/usr/include/asm/prctl.h`, `/usr/include/linux/sched.h`, `/usr/include/elf.h`.

## Investigación previa (resumen con enlaces)

- Traducción dinámica, TCG y modo usuario: Bellard, USENIX 2005, `https://www.usenix.org/legacy/event/usenix05/tech/freenix/full_papers/bellard/bellard_html/`.
- Flags perezosas de QEMU: `https://patchew.org/QEMU/20240611142524.83762-1-pbonzini@redhat.com/20240611142524.83762-8-pbonzini@redhat.com`.
- Caché, enlazado, saltos indirectos y trazas en DynamoRIO (300× → ~1.1×): `https://www.burningcutlery.com/derek/docs/DynamoRIO-tutorial-2015.pdf`; caché acotada: `https://commit.csail.mit.edu/papers/2005/RIO-cacheconscap.pdf`.
- Diferencias de decodificación y de flags entre fabricantes (`f3 47 90`, AF, RCPPS): `https://arxiv.org/pdf/2608.13042`; BSF/BSR: `https://lists.xen.org/archives/html/xen-devel/2025-04/msg01797.html`; rotaciones: `https://patchew.org/Xen/367ba117-f376-433b-bd70-586d7519d94c@suse.com/04602af0-9a03-4d43-beb3-28b76e2cad33@suse.com`.
- Pruebas aleatorias contra hardware: risu, `https://git-us.linaro.org/people/pmaydell/risu.git/tree/risu.c`; reducción en TestRIG, `https://riscv.org/blog/2025/02/testrig-randomized-testing-of-risc-v-cpus/`.
- Yama/ptrace: `https://www.kernel.org/doc/Documentation/admin-guide/LSM/Yama.rst`.
- Comparables (FEX, Box64, Rosetta 2) y huecos: en la investigación adjunta del laboratorio; no se usan como fuente de ningún número de este proyecto.
