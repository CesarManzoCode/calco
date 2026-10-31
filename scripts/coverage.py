"""Coverage (contract 6): disassembles the generated test ELFs with `objdump -d -M intel`,
takes the instructions between the `case_N` and `case_N_end` symbols (the generated
sequences, not the state load or the dump routine), normalizes their names and prints
the mnemonics of `isa/scope.tsv` that no case covers. The mnemonics of
`harness/exclusions.tsv` are left out: the generator never emits them (contract 4.2).
Exits 0 only if nothing is uncovered.

It uses the ELFs of random_compare.py in `build/random/elf/` (or `--elf-dir`); if there
are none it builds cases 0…N-1 into `build/coverage/elf/`.

    python3 scripts/coverage.py [--n N] [--elf-dir DIR]
"""

import argparse
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import common

sys.path.insert(0, str(common.ROOT / "harness"))
import build  # noqa: E402
import gen  # noqa: E402

# Prefixes objdump prints as separate words before the mnemonic.
PREFIXES = {"lock", "rep", "repz", "repnz", "repe", "repne", "data16", "data32", "addr32",
            "notrack", "bnd", "cs", "ds", "es", "ss", "fs", "gs"}
# Size suffixes objdump may add to a name of the scope (`pushfq`, `movsb`, `xlatb`).
SUFFIXES = ("64", "b", "w", "l", "d", "q")
SYMBOL = re.compile(r"^[0-9a-f]+ <([^>]+)>:$")
CASE = re.compile(r"case_\d+")
# objdump prints CMPPS/CMPSS/CMPPD/CMPSD with imm8 0-7 as their predicate alias.
CMP_PREDICATE = re.compile(r"cmp(?:eq|lt|le|unord|neq|nlt|nle|ord)(ps|ss|pd|sd)")


def normalize(text, scope):
    """Scope name of an instruction as objdump prints it, or its bare mnemonic."""
    words = [w for w in text.split() if w not in PREFIXES and not w.startswith("rex")]
    if not words:
        return None
    mn = words[0]
    if m := CMP_PREDICATE.fullmatch(mn):
        mn = "cmp" + m[1]
    if mn in scope:
        return mn
    for suffix in SUFFIXES:
        if mn.endswith(suffix) and mn[: -len(suffix)] in scope:
            return mn[: -len(suffix)]
    return mn


def mnemonics(elf, scope):
    """Normalized mnemonics of the generated sequences of a test ELF."""
    proc = subprocess.run(["objdump", "-d", "-M", "intel", "--no-show-raw-insn", str(elf)],
                          capture_output=True, text=True)
    if proc.returncode != 0:
        common.fail(f"objdump {elf}: {proc.stderr}")
    found, inside = set(), False
    for line in proc.stdout.splitlines():
        if m := SYMBOL.match(line):
            inside = bool(CASE.fullmatch(m[1]))
            continue
        parts = line.split("\t")
        if inside and len(parts) >= 2 and parts[0].strip().endswith(":"):
            text = parts[-1].split("#")[0].strip()
            if text and text != "(bad)" and (mn := normalize(text, scope)):
                found.add(mn)
    return found


def build_elfs(n, out):
    def one(number, first):
        cases = build.batch(first, min(build.BATCH, n - first))
        return build.build(cases, out, f"batch_{number:04d}")

    with ThreadPoolExecutor(max_workers=os.cpu_count() or 1) as pool:
        return list(pool.map(lambda a: one(*a), enumerate(range(0, n, build.BATCH))))


def main():
    common.require_non_root()
    p = argparse.ArgumentParser()
    p.add_argument("--n", type=int, default=100_000)
    p.add_argument("--elf-dir", type=Path, default=common.BUILD / "random" / "elf")
    args = p.parse_args()

    elfs = sorted(e for e in args.elf_dir.glob("batch_*") if e.suffix == "")
    if not elfs:
        elfs = build_elfs(args.n, common.BUILD / "coverage" / "elf")
    scope = gen.load_scope()
    exclusions = gen.load_exclusions()
    with ThreadPoolExecutor(max_workers=os.cpu_count() or 1) as pool:
        covered = set().union(*pool.map(lambda e: mnemonics(e, scope), elfs))

    wanted = sorted(set(scope) - exclusions)
    missing = [mn for mn in wanted if mn not in covered]
    for mn in missing:
        print(f"sin cubrir: {mn} ({scope[mn].ext})")
    print(f"ELF: {len(elfs)}, excluidas: {len(set(scope) & exclusions)}")
    print(f"cobertura: {len(wanted) - len(missing)}/{len(wanted)}")
    if missing:
        common.fail(f"{len(missing)} mnemonics of the scope without coverage")


if __name__ == "__main__":
    main()
