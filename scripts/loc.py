"""Harness smaller than translator (contract 6): counts lines that are not empty and not
only a comment, and prints `traductor: <a>` and `verificacion: <b>`; fails if b >= a.

Translator: `src/**/*.rs` except the `tests.rs` files. Verification: `harness/**`,
`scripts/**`, `guests/**`, `tests/**` and `src/**/tests.rs`, files `.py .rs .c .S .s .sh`.
`.tsv` tables and the inputs under `targets/` do not count.

A line is only a comment if, stripped, it starts with the comment marker of its language
or is a one-line `/* … */`: `//` in Rust and C; `#` in Python, shell and assembly (`.s`,
`.S`, where GNU as reads `#` as a comment). In Rust a `#[…]` attribute and in C a `#include`
are code, not comments.

    python3 scripts/loc.py
"""

import sys
from pathlib import Path

import common

EXTENSIONS = {".py", ".rs", ".c", ".S", ".s", ".sh"}
VERIFICATION_DIRS = ("harness", "scripts", "guests", "tests")
LINE_COMMENT = {".rs": "//", ".c": "//", ".py": "#", ".sh": "#", ".s": "#", ".S": "#"}
BLOCK_COMMENT = {".rs", ".c", ".S"}


def is_comment(line, suffix):
    """Whether the stripped `line` of a `suffix` file is only a comment."""
    if line.startswith(LINE_COMMENT[suffix]):
        return True
    return (suffix in BLOCK_COMMENT and line.startswith("/*") and line.endswith("*/")
            and "*/" not in line[2:-2])


def count(text, suffix):
    """Counted lines of `text`, the contents of a `suffix` file."""
    lines = (line.strip() for line in text.splitlines())
    return sum(1 for line in lines if line and not is_comment(line, suffix))


def side(rel):
    """`traductor`, `verificacion` or None for a path relative to the repository root."""
    parts = rel.parts
    if not parts or rel.suffix not in EXTENSIONS:
        return None
    if parts[0] == "src" and rel.suffix == ".rs":
        return "verificacion" if rel.name == "tests.rs" else "traductor"
    if parts[0] in VERIFICATION_DIRS:
        return "verificacion"
    return None


def totals(root=common.ROOT):
    out = {"traductor": 0, "verificacion": 0}
    for top in ("src", *VERIFICATION_DIRS):
        for path in sorted((root / top).rglob("*")):
            rel = path.relative_to(root)
            kind = side(rel) if path.is_file() else None
            if kind:
                out[kind] += count(path.read_text(errors="replace"), path.suffix)
    return out


def main():
    common.require_non_root()
    t = totals()
    print(f"traductor: {t['traductor']}")
    print(f"verificacion: {t['verificacion']}")
    sys.exit(0 if t["verificacion"] < t["traductor"] else 1)


if __name__ == "__main__":
    main()
