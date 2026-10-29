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
stellar contract deploy \
  --wasm "$WASM" \
  --source "$IDENTITY" \
  --network "$NETWORK"
