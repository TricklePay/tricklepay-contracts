#!/usr/bin/env bash
#
# Build and deploy the stream contract to a Stellar network.
#
# Usage:
#   ./scripts/deploy.sh <identity-name>
#
# Requires the Stellar CLI (https://developers.stellar.org/docs/tools/cli) and a
# funded identity created with `stellar keys generate`. The network defaults to
# testnet and can be overridden with the NETWORK environment variable.

set -euo pipefail

IDENTITY="${1:-}"
if [ -z "$IDENTITY" ]; then
  echo "usage: $0 <identity-name>" >&2
  exit 1
fi

NETWORK="${NETWORK:-testnet}"
# soroban-sdk requires wasm32v1-none on Rust 1.82+; wasm32-unknown-unknown
# enables wasm features the Soroban environment does not support.
WASM_TARGET="wasm32v1-none"
WASM="target/${WASM_TARGET}/release/tricklepay_stream.wasm"

echo "Building optimized WASM..."
cargo build --locked --release --target "$WASM_TARGET"

echo "Deploying to ${NETWORK} as '${IDENTITY}'..."
CONTRACT_ID=$(stellar contract deploy \
  --wasm "$WASM" \
  --source "$IDENTITY" \
  --network "$NETWORK")

if command -v sha256sum >/dev/null 2>&1; then
  WASM_HASH=$(sha256sum "$WASM" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
  WASM_HASH=$(shasum -a 256 "$WASM" | awk '{print $1}')
else
  echo "Error: Neither sha256sum nor shasum is available." >&2
  exit 1
fi

echo "=========================================="
echo "Deployment Successful!"
echo "Contract ID : $CONTRACT_ID"
echo "WASM File   : $WASM"
echo "WASM Hash   : sha256:$WASM_HASH"
echo "=========================================="
