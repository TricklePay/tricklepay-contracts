#!/usr/bin/env bash
#
# Extend the TTL of a stream entry.
#
# Usage:
#   ./scripts/extend-ttl.sh <contract-id> <stream-id> <network>
#
# Arguments:
#   contract-id  The deployed stream contract address
#   stream-id    The numeric stream ID to extend
#   network      Either 'testnet' or 'mainnet'
#
# Requirements:
#   - Stellar CLI version 25.2.0 or compatible
#   - A funded identity configured for the specified network
#   - The stream entry must exist (not archived)
#
# Example:
#   ./scripts/extend-ttl.sh CDXYZ...ABC 42 testnet
#
# Output:
#   Prints the new TTL in ledgers after successful extension.

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

error() {
    echo -e "${RED}Error: $1${NC}" >&2
    exit 1
}

info() {
    echo -e "${GREEN}$1${NC}"
}

warn() {
    echo -e "${YELLOW}$1${NC}"
}

# Validate arguments
if [ $# -ne 3 ]; then
    error "Usage: $0 <contract-id> <stream-id> <network>

Arguments:
  contract-id  The deployed stream contract address
  stream-id    The numeric stream ID to extend
  network      Either 'testnet' or 'mainnet'

Example:
  $0 CDXYZ...ABC 42 testnet"
fi

CONTRACT_ID="$1"
STREAM_ID="$2"
NETWORK="$3"

# Validate contract ID format (basic check for non-empty and reasonable length)
if [ -z "$CONTRACT_ID" ]; then
    error "Contract ID cannot be empty"
fi

if [ ${#CONTRACT_ID} -lt 10 ]; then
    error "Contract ID appears too short (expected 56 characters)"
fi

# Validate stream ID is a positive integer
if ! [[ "$STREAM_ID" =~ ^[0-9]+$ ]]; then
    error "Stream ID must be a positive integer, got: $STREAM_ID"
fi

# Validate network
if [ "$NETWORK" != "testnet" ] && [ "$NETWORK" != "mainnet" ]; then
    error "Network must be either 'testnet' or 'mainnet', got: $NETWORK"
fi

# Set RPC URL based on network
if [ "$NETWORK" = "testnet" ]; then
    RPC_URL="https://soroban-testnet.stellar.org"
elif [ "$NETWORK" = "mainnet" ]; then
    RPC_URL="https://soroban-mainnet.stellar.org"
fi

# Check if stellar CLI is available
if ! command -v stellar &> /dev/null; then
    error "Stellar CLI not found. Install it with:
  cargo install --locked stellar-cli --version 25.2.0"
fi

# Warn about CLI version
STELLAR_VERSION=$(stellar version 2>/dev/null || echo "unknown")
if ! echo "$STELLAR_VERSION" | grep -q "25.2.0"; then
    warn "Warning: Expected Stellar CLI version 25.2.0, found: $STELLAR_VERSION"
    warn "Proceeding anyway, but CLI flag or XDR compatibility issues may occur."
fi

# Construct the ledger key for Stream(id)
# The key format is: [{"symbol":"Stream"},{"u64":<STREAM_ID>}]
LEDGER_KEY="[{\"symbol\":\"Stream\"},{\"u64\":$STREAM_ID}]"

info "Extending TTL for stream $STREAM_ID on contract $CONTRACT_ID ($NETWORK)..."
info "Ledger key: $LEDGER_KEY"

# Extend the TTL
# The --ledgers-to-extend value is set to ENTRY_TTL (518400 ledgers ≈ 30 days)
LEDGERS_TO_EXTEND=518400

# Run the extend command
# Note: This requires a funded source account/identity configured for the network
if ! stellar contract extend \
    --id "$CONTRACT_ID" \
    --network "$NETWORK" \
    --rpc-url "$RPC_URL" \
    --key "$LEDGER_KEY" \
    --ledgers-to-extend "$LEDGERS_TO_EXTEND" 2>&1; then
    
    error "Failed to extend TTL. Possible causes:
  - Stream ID $STREAM_ID does not exist or is archived (restore it first)
  - No funded identity configured for network '$NETWORK'
  - Contract ID is incorrect
  - Network connectivity issue

To restore an archived stream, see docs/TTL_RESTORE.md"
fi

# Query the new TTL (this is an estimate based on the extension)
info "✓ TTL extended successfully!"
info "New TTL: ~$LEDGERS_TO_EXTEND ledgers (≈ 30 days at 5-second ledger close)"
info ""
info "Note: The actual TTL may vary if the entry was already extended recently."
info "      To confirm, query the stream with: stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- get_stream --id $STREAM_ID"

exit 0
