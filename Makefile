# Single check command: format, types and tests. Long comparison scripts run in the K checkers.
.PHONY: check

check:
	@if [ -f Cargo.toml ]; then cargo fmt --check && cargo test --release; else echo "check: no Cargo.toml yet"; fi
	@if ls harness/test_*.py >/dev/null 2>&1; then python3 -m unittest discover -s harness -p 'test_*.py'; else echo "check: no harness tests yet"; fi
