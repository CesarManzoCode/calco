"""Test ELF of a batch (contract 4.1): one static binary without libc that, for each case,
loads the initial state, runs the sequence and calls `dump`, which writes the 1280-byte
record to stdout. After the last case it calls `exit_group(0)`.

The same binary runs natively, under calco and under qemu-x86_64. Addresses are fixed
(non-PIE `ld -static`), so RIP, RSP and stored return addresses agree in the three runs.
Next to the ELF goes `<name>.mask.json` with the undefined flags of each case.
"""

import json
import subprocess
from pathlib import Path

import gen

BATCH = 500
STACK_AREA = 4096
STACK_TOP = 2048  # initial RSP of every case, as an offset in the stack area
# Initial state block of a case: buffer, FXSAVE image (16-aligned), RFLAGS, GPR.
INIT_BUF, INIT_FX, INIT_RFLAGS, INIT_GPR, INIT_SIZE = 0, 256, 768, 776, 960

DUMP = """\
dump:
	mov [rip + saved_rsp], rsp
	lea rsp, [rip + dump_stack + 256]
	pushfq
	pop qword ptr [rip + rec + 152]
{gpr_stores}
	mov rax, [rip + saved_rsp]
	mov rcx, [rax]
	mov [rip + rec + 144], rcx
	lea rcx, [rax + 8]
	mov [rip + rec + 48], rcx
	fxsave64 [rip + rec + 160]
	cld
	lea rsi, [rip + buf]
	lea rdi, [rip + rec + 672]
	mov ecx, 32
	rep movsq
	lea rsi, [rip + stack_area + {window}]
	lea rdi, [rip + rec + 928]
	mov ecx, 32
	rep movsq
	mov eax, [rip + counter]
	lea rcx, [rip + case_ids]
	mov eax, [rcx + rax*4]
	mov [rip + rec + 8], eax
	inc dword ptr [rip + counter]
	mov eax, 1
	mov edi, 1
	lea rsi, [rip + rec]
	mov edx, 1280
	syscall
	cmp rax, 1280
	jne 1f
	mov rsp, [rip + saved_rsp]
	ret
1:	mov eax, 231
	mov edi, 3
	syscall
"""


def _quads(blob):
    return [f"\t.quad {int.from_bytes(blob[k:k + 8], 'little'):#x}" for k in range(0, len(blob), 8)]


def case_text(case):
    """Assembly of one case: state load, items with their labels, `call dump`."""
    n = case.index
    out = [
        f"# case {n}",
        "\tcld",
        f"\tlea rsp, [rip + stack_area + {STACK_TOP}]",
        "\tlea rdi, [rip + stack_area]",
        "\txor eax, eax",
        f"\tmov ecx, {STACK_AREA // 8}",
        "\trep stosq",
        f"\tlea rsi, [rip + init_{n}]",
        "\tlea rdi, [rip + buf]",
        "\tmov ecx, 32",
        "\trep movsq",
        f"\tfxrstor64 [rip + init_{n} + {INIT_FX}]",
        f"\tpush qword ptr [rip + init_{n} + {INIT_RFLAGS}]",
        "\tpopfq",
    ]
    out += [f"\tmov {r}, [rip + init_{n} + {INIT_GPR + 8 * k}]"
            for k, r in enumerate(gen.R64) if k != gen.RSP]
    ids = [item.id for item in case.items]
    # `case_N` and `case_N_end` are symbols, so `objdump` shows where the sequence is
    # (scripts/coverage.py counts only what lies between them).
    out.append(f"case_{n}:")
    for item in case.items:
        target = next((f".L{n}_{i}" for i in ids if i >= (item.target or 0)), f".L{n}_end")
        out.append(f".L{n}_{item.id}:\t# {item.mnemonic}")
        out += ["\t" + line.replace("{T}", target) for line in item.lines]
    out += [f".L{n}_end:", f"case_{n}_end:", "\tcall dump"]
    return out


def init_text(case):
    n = case.index
    block = bytearray(INIT_SIZE)
    block[INIT_BUF:INIT_FX] = case.buf
    block[INIT_FX:INIT_RFLAGS] = case.fxsave
    block[INIT_RFLAGS:INIT_GPR] = case.rflags.to_bytes(8, "little")
    for k, v in enumerate(case.gpr):
        block[INIT_GPR + 8 * k : INIT_GPR + 8 * k + 8] = v.to_bytes(8, "little")
    return ["\t.balign 64", f"init_{n}:", *_quads(block)]


def assembly(cases):
    out = ["\t.intel_syntax noprefix", "\t.text", "\t.globl _start", "_start:"]
    for case in cases:
        out += case_text(case)
    out += ["\tmov eax, 231", "\txor edi, edi", "\tsyscall"]
    gpr_stores = "\n".join(f"\tmov [rip + rec + {16 + 8 * k}], {r}"
                           for k, r in enumerate(gen.R64) if k != gen.RSP)
    out.append(DUMP.format(gpr_stores=gpr_stores, window=STACK_TOP - 256))
    out += ["\t.data", "\t.balign 64", "rec:", '\t.ascii "CALCODMP"', "\t.zero 1272"]
    out += ["\t.balign 4", "case_ids:", *(f"\t.long {c.index}" for c in cases)]
    for case in cases:
        out += init_text(case)
    out += ["\t.bss", "\t.balign 64", "buf:", f"\t.zero {gen.BUF_SIZE}", "\t.balign 64",
            "stack_area:", f"\t.zero {STACK_AREA}", "dump_stack:", "\t.zero 256",
            "saved_rsp:", "\t.zero 8", "counter:", "\t.zero 4"]
    return "\n".join(out) + "\n"


def build(cases, out_dir, name):
    """Writes `<name>.s`, assembles it with `as --64`, links it with `ld -static -nostdlib`
    and writes `<name>.mask.json`. Returns the path of the ELF."""
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    src, obj, elf = (out_dir / f"{name}{ext}" for ext in (".s", ".o", ""))
    src.write_text(assembly(cases))
    for cmd in (["as", "--64", "-o", obj, src], ["ld", "-static", "-nostdlib", "-o", elf, obj]):
        proc = subprocess.run([str(c) for c in cmd], capture_output=True, text=True)
        if proc.returncode != 0:
            raise RuntimeError(f"{cmd[0]} failed on {src}:\n{proc.stderr}")
    masks = {str(c.index): sorted(c.undefined) for c in cases}
    (out_dir / f"{name}.mask.json").write_text(json.dumps(masks, indent=0) + "\n")
    return elf


def batch(first, count=BATCH):
    """Cases `first … first + count - 1`."""
    return [gen.generate(i) for i in range(first, first + count)]
