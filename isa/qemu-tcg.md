# Instrucciones de las pruebas tests/tcg de QEMU

Lista de las instrucciones que usan, en 64 bits, `test-x86_64` (`test-i386.c`, `test-i386.h`, `test-i386-shift.h`, `test-i386-muldiv.h`) y `sha512-sse` (`sha512.c` con `-msse4.1 -O3`), y su lugar en el alcance. Fuentes de QEMU v11.1.2 descargadas con `python3 scripts/fetch.py qemu-tests` (sha256 en `scripts/pins.json`).

## Cómo se obtuvo

[hecho] En la máquina de referencia (gcc 16.2.1, binutils 2.47), el 2026-10-30, se compilaron los objetos sin enlazar (solo el código de las pruebas, sin libc) y se contaron los mnemónicos:

```sh
gcc -Wall -O0 -g -fno-strict-aliasing -c -o test-i386.o build/third_party/qemu-tests/i386/test-i386.c
gcc -O3 -msse4.1 -c -o sha512.o build/third_party/qemu-tests/multiarch/sha512.c
objdump -d -M intel --no-show-raw-insn test-i386.o   # y lo mismo con sha512.o
```

Además se leyó el código: en 64 bits `main` no llama a `test_bcd`, `test_segs`, `test_code16`, `test_vm86`, `test_exceptions`, `test_self_modifying_code` ni `test_single_step` (están tras `#if !defined(__x86_64__)` o `TEST_SEGS`), y el bloque de `lcall`/`ljmp` de `test_misc` está en `#if 0`.

## test-x86_64 (código que se ejecuta)

| Grupo | Mnemónicos (`objdump -M intel`) | Notas |
| --- | --- | --- |
| Aritmética y lógica | `add sub adc sbb and or xor cmp test inc dec neg not` | Formas b, w, l, q con `pushf`/`popf` alrededor. |
| Desplazamientos y rotaciones | `shl shr sar rol ror rcl rcr shld shrd` | Cuenta inmediata y en `cl`. |
| Bits | `bt bts btr btc bsf bsr popcnt bswap` | `popcnt` de 16, 32 y 64 bits. |
| `tzcnt`, `lzcnt` | `tzcnt lzcnt` (`F3 0F BC/BD`) | No son filas del alcance: son la codificación heredada `rep bsf`/`rep bsr` que el contrato (3.4) acepta y que el anfitrión ejecuta igual que en nativo. |
| Multiplicación y división | `mul imul div idiv` | `imul` de uno, dos y tres operandos (con inmediato) en w, l, q. |
| Saltos y condiciones | `jcc` (las 16), `setcc` (las 16), `cmovcc` (las 16) en w, l, q | |
| Bucles | `loop loope loopne jrcxz` | Con prefijo `67` (`loopl`, `jecxz`: cuenta en ECX). |
| Intercambio | `xchg xadd cmpxchg cmpxchg8b` | Registro y memoria. |
| Cadenas | `movs stos lods scas cmps` | b, w, l, q, con y sin `rep`/`repz`/`repnz`, con DF a 0 y a 1 (`std`, `cld`). |
| Pila y flags | `push pop pushf popf enter leave` | `pop` a memoria (`pop [rsp]`); `enter` de 64 y de 16 bits (`66 C8`, `enterw`). |
| Conversiones | `cbw cwde cdqe cwd cdq cqo` | |
| Varias | `xlat lea mov movabs movzx movsx nop` | `lea` con dirección de 32 bits (prefijo `67`) y RIP-relativo. |
| x87 | `fld fst fstp fist fistp fbld fbstp fld1 fldl2t fldl2e fldpi fldlg2 fldln2 fldz fcom fucom fcomi fucomi fxam frndint fcmovb fcmove fcmovbe fcmovnb fcmovne fcmovnbe fcmovu fcmovnu ffree fninit fldcw fnstcw fnstsw fwait fnstenv fldenv fnsave frstor` | `fstcw`/`fstsw` son `fwait` + `fnstcw`/`fnstsw`. Las cuatro de entorno también con prefijo `66` (formato de 16 bits). |
| SSE2 escalar (del código C) | `movsd movss movq movapd pxor addsd subsd mulsd divsd cvtsi2sd cvtsd2ss cvtss2sd` | Las emite gcc para los `double` de `printf` y de las pruebas de coma flotante. |

En el objeto también aparecen `cli`, `in`, `out`, `ins`, `outs`, `hlt`, `int`, `int3`, `ud2`, `lock nop` y `retfq`, pero solo en funciones que `main` no llama en 64 bits (`test_exceptions`) o en `func_lret`, que solo usa el bloque en `#if 0`.

## sha512-sse (lo que gcc emitió con `-msse4.1 -O3`)

- SSE/SSE2: `movaps movups movdqa movdqu movd movq pand por pxor paddb paddq psubd psubusb pcmpeqb pcmpeqd pcmpgtd pminub pshufd pslld psllq psrld psrldq psrlq psrlw packuswb punpckhbw punpcklbw punpckhqdq punpcklqdq`.
- SSE3: `movddup`.
- SSSE3: `pshufb palignr`.
- SSE4.1: `packusdw pblendvb pblendw pextrq pinsrq pminud pmovzxbw pmovzxwd`.
- Base: `add sub and or xor rol ror shl shr bswap cmp test cmova cmovb cmovg sete setne movabs movzx movsx xchg lea` y relleno `nop` con prefijos `66`/`2E`.

No aparece ninguna instrucción fuera de SSE4.1: ni SSE4.2, ni AVX, ni `pcmpgtq`, `pmulld` o `roundpd`, aunque gcc podría emitirlas con otras banderas o en otro código.

## Resultado de la revisión

- [hecho] Todos los mnemónicos de las dos listas, salvo `tzcnt` y `lzcnt`, son filas de `isa/scope.tsv`, y `src/decode/table.rs` acepta sus codificaciones (los prefijos `66` y `67` los trata el decodificador de forma general). `tzcnt`/`lzcnt` entran por la regla de codificaciones heredadas del contrato 3.4 (`0F BC/BD` con `F3`).
- [diseño] No se añadieron filas: no faltaba ninguna instrucción del alcance. Las variantes por prefijo (`enter` de 16 bits, `loop`/`jrcxz` con `67`, entorno x87 de 16 bits, `lea` con dirección de 32 bits) no se añadieron como formas de `isa/scope.tsv` porque esas formas son las plantillas del generador aleatorio y el generador no las sabe emitir; quedan cubiertas por `test-x86_64`.
- [hipótesis] La parte de libc enlazada de forma estática elige rutinas por el CPUID que ve (contrato 3.9), así que bajo calco ejecuta variantes SSE2/SSE4 distintas de las AVX2 que usa en nativo. Que `printf` y libm den la misma salida con esas variantes lo comprueba `scripts/qemu_tcg.py` al cierre de K1; no se ha medido.
