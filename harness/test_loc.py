"""Tests of the line counting rules of scripts/loc.py (contract 6) on fixed texts."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
import loc  # noqa: E402


class Count(unittest.TestCase):
    def test_empty_lines(self):
        self.assertEqual(loc.count("\n   \n\t\nx = 1\n\n", ".py"), 1)

    def test_rust_comments(self):
        text = "// line\n  /// doc\n//! module\n/* block */\nfn f() {} // trailing\nlet x = 1;\n"
        self.assertEqual(loc.count(text, ".rs"), 2)

    def test_rust_attribute_is_code(self):
        self.assertEqual(loc.count("#[derive(Debug)]\n#![allow(unused)]\n", ".rs"), 2)

    def test_python_and_shell_comments(self):
        text = "#!/usr/bin/env python3\n# comment\n    # indented\nprint(1)  # trailing\n"
        self.assertEqual(loc.count(text, ".py"), 1)
        self.assertEqual(loc.count("# c\necho hi\n", ".sh"), 1)

    def test_c(self):
        text = "#include <stdio.h>\n/* one line */\n// line\nint main(void) { return 0; }\n"
        self.assertEqual(loc.count(text, ".c"), 2)

    def test_block_comment_spanning_lines_counts(self):
        # Only one-line `/* … */` is a comment line; `/* a */ x /* b */` is code.
        self.assertEqual(loc.count("/* a */ x = 1; /* b */\n/* open\n", ".c"), 2)

    def test_assembly(self):
        self.assertEqual(loc.count("# comment\n\tmov rax, 1\n/* c */\n", ".S"), 1)
        self.assertEqual(loc.count("# comment\n\tmov rax, 1\n", ".s"), 1)


class Side(unittest.TestCase):
    def test_translator_and_verification(self):
        for rel in ("src/main.rs", "src/jit/translate.rs"):
            self.assertEqual(loc.side(Path(rel)), "traductor", rel)
        for rel in ("src/jit/tests.rs", "src/cpu/tests.rs", "harness/gen.py", "scripts/loc.py",
                    "guests/hello.c", "guests/asm/basic.S", "tests/jit_basic.rs", "scripts/x.sh",
                    "guests/x.s"):
            self.assertEqual(loc.side(Path(rel)), "verificacion", rel)

    def test_not_counted(self):
        for rel in ("harness/mutants.tsv", "isa/scope.tsv", "targets/lua/a.lua",
                    "targets/x/gen.py", "guests/avx.expected", "Cargo.toml", "README.md"):
            self.assertIsNone(loc.side(Path(rel)), rel)


class Totals(unittest.TestCase):
    def test_tree(self):
        import tempfile

        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            files = {
                "src/main.rs": "// c\nfn main() {}\n\n",
                "src/jit/mod.rs": "#[cfg(test)]\nmod tests;\n",
                "src/jit/tests.rs": "#[test]\nfn t() {}\n",
                "harness/a.py": "# c\nx = 1\n",
                "harness/t.tsv": "a\tb\n",
                "targets/x.py": "y = 2\n",
            }
            for rel, text in files.items():
                (root / rel).parent.mkdir(parents=True, exist_ok=True)
                (root / rel).write_text(text)
            self.assertEqual(loc.totals(root), {"traductor": 3, "verificacion": 3})


if __name__ == "__main__":
    unittest.main()
