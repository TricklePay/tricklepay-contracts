#!/usr/bin/env bash
#
# Compare the built release WASM against the checked-in size budget.
#
# The budget lives in wasm-size-budget.txt at the repository root. If the
# artifact is larger, the script prints both numbers and fails; raising the
# budget is an intentional edit that belongs in the same pull request.

set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
cd "$ROOT_DIR"

WASM="target/wasm32v1-none/release/tricklepay_stream.wasm"
BUDGET_FILE="${WASM_SIZE_BUDGET_FILE:-wasm-size-budget.txt}"

if [[ ! -f "$WASM" ]]; then
    echo "Error: missing Wasm artifact: $WASM" >&2
    echo "run 'make wasm' before checking the size budget" >&2
    exit 1
fi

if [[ ! -f "$BUDGET_FILE" ]]; then
    echo "Error: missing size budget file: $BUDGET_FILE" >&2
    exit 1
fi

budget="$(awk 'NF && $1 !~ /^#/ { print $1; exit }' "$BUDGET_FILE")"
if [[ ! "$budget" =~ ^[0-9]+$ ]]; then
    echo "Error: $BUDGET_FILE must contain a size in bytes, got: ${budget:-<empty>}" >&2
    exit 1
fi

size="$(wc -c < "$WASM" | tr -d ' ')"

if (( size > budget )); then
    echo "Error: release WASM exceeds the size budget." >&2
    echo "  budget: $budget bytes" >&2
    echo "  actual: $size bytes" >&2
    echo "  over:   $((size - budget)) bytes" >&2
    echo "If the growth is intentional, raise $BUDGET_FILE in this pull request." >&2
    exit 1
fi

echo "WASM size: $size bytes (budget: $budget bytes, $((budget - size)) bytes headroom)"
