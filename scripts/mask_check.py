"""Mask file with references (contract 6): every mnemonic of `isa/scope.tsv` with a
non-empty `flags_undefined` must be in `harness/undefined_flags.tsv` with a non-empty
`ref` and the same flags, and the header of that file must name the manual revision it
was transcribed from. Prints `mascara-con-referencias`.
"""

import re
import sys

import common

sys.path.insert(0, str(common.ROOT / "harness"))
import gen  # noqa: E402


def problems():
    scope = gen.load_scope()
    table, header = gen.load_undefined()
    found = []
    if not any(re.search(r"24594.*(revisi[oó]n|rev)\s*\d", line) for line in header):
        found.append("header: no AMD64 APM (24594) revision")
    for mn, row in scope.items():
        if not row.undefined:
            continue
        if mn not in table:
            found.append(f"{mn}: undefined {' '.join(sorted(row.undefined))} without a row")
            continue
        flags, ref = table[mn]
        if not ref:
            found.append(f"{mn}: empty ref")
        if flags != row.undefined:
            found.append(f"{mn}: scope says {' '.join(sorted(row.undefined))}, "
                         f"undefined_flags.tsv says {' '.join(sorted(flags))}")
    for mn in sorted(set(table) - set(scope)):
        found.append(f"{mn}: in undefined_flags.tsv but not in the scope")
    return found


def main():
    common.require_non_root()
    found = problems()
    for line in found:
        print(line)
    if found:
        common.fail(f"{len(found)} problems in the mask table")
    print("mascara-con-referencias")


if __name__ == "__main__":
    main()
