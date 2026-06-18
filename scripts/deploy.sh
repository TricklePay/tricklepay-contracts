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
WASM="target/wasm32-unknown-unknown/release/tricklepay_stream.wasm"

echo "Building optimized WASM..."
cargo build --release --target wasm32-unknown-unknown

echo "Deploying to ${NETWORK} as '${IDENTITY}'..."
stellar contract deploy \
  --wasm "$WASM" \
  --source "$IDENTITY" \
  --network "$NETWORK"
