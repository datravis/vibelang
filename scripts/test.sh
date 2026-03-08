#!/usr/bin/env bash
# Run all VibeLang tests: cargo tests + example compilation checks.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$SCRIPT_DIR/.."
COMPILER="$ROOT/compiler"

echo "=== VibeLang Test Suite ==="
echo

# 1. Build the compiler
echo "--- Building compiler ---"
(cd "$COMPILER" && cargo build --quiet)
echo "OK"
echo

# 2. Run Rust unit + integration tests
echo "--- Running cargo tests ---"
(cd "$COMPILER" && cargo test --quiet)
echo "OK"
echo

# 3. Compile each example with the vibe CLI (lex + parse + typecheck)
echo "--- Checking examples (vibe check) ---"
VIBE="$COMPILER/target/debug/vibe"
EXAMPLES_DIR="$ROOT/examples"
pass=0
fail=0
for f in "$EXAMPLES_DIR"/*.vibe; do
    name=$(basename "$f")
    if "$VIBE" check "$f" > /dev/null 2>&1; then
        echo "  PASS  $name"
        pass=$((pass + 1))
    else
        echo "  FAIL  $name"
        fail=$((fail + 1))
    fi
done
echo
echo "Examples: $pass passed, $fail failed"

if [ "$fail" -gt 0 ]; then
    echo "FAILED"
    exit 1
fi

echo
echo "=== All tests passed ==="
