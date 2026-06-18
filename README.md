# TricklePay Contracts

Soroban smart contracts for TricklePay, a token streaming protocol on Stellar.

A stream locks a sum of tokens from a sender and releases them to a recipient
linearly over time. The recipient can withdraw whatever has vested at any
moment; the sender can cancel and reclaim only the portion that has not yet
vested. This is the on-chain primitive behind payroll, vesting, grants, and
subscriptions, where value should move continuously rather than in lump sums.

This repository holds the `stream` contract and its test suite. The indexer and
web client that build on it live in separate repositories; see
[Related repositories](#related-repositories).

## How a stream works

A stream is defined by a total amount and a window of time:

- **Start and end** bound the linear release. At the start nothing has vested;
  at the end the full amount has vested; in between the vested amount grows in
  proportion to elapsed time.
- **Cliff** (optional) is a point before which nothing can be withdrawn. When
  the cliff is reached, everything accrued since the start unlocks at once and
  vesting continues linearly from there. A stream with `cliff == start` has no
  cliff.
- **Withdraw** sends the recipient whatever has vested minus what they have
  already taken.
- **Cancel** stops a stream early. The recipient keeps everything vested up to
  that moment; the unvested remainder is refunded to the sender. A cancelled
  stream's vested balance stays claimable.

All amounts are in the token's smallest unit. All times are Unix timestamps in
seconds, matching the ledger clock.

## Contract interface

| Function | Caller | Description |
| --- | --- | --- |
| `create_stream(sender, recipient, token, total_amount, start_time, end_time, cliff_time) -> u64` | sender | Locks `total_amount` and opens a stream, returning its id. |
| `withdraw(id) -> i128` | recipient | Transfers the vested, unwithdrawn balance to the recipient. |
| `cancel(id) -> i128` | sender | Refunds the unvested remainder to the sender and freezes the stream. |
| `get_stream(id) -> Stream` | anyone | Returns the full stream record. |
| `withdrawable(id) -> i128` | anyone | Amount the recipient can withdraw right now. |
| `vested(id) -> i128` | anyone | Total vested so far, including what was withdrawn. |
| `status(id) -> StreamStatus` | anyone | `Pending`, `Streaming`, `Completed`, or `Cancelled`. |
| `stream_count() -> u64` | anyone | Number of streams created; ids run from 0 upward. |

The contract publishes `Created`, `Withdrawn`, and `Cancelled` events, each
carrying the parties as topics so an indexer can filter streams by sender or
recipient.

## Building

A recent stable Rust toolchain with the `wasm32-unknown-unknown` target is
required; the pinned versions are in `rust-toolchain.toml`.

```bash
# Native build and the full test suite
cargo test

# Optimized WASM ready to deploy
cargo build --release --target wasm32-unknown-unknown
```

The release artifact is written to
`target/wasm32-unknown-unknown/release/tricklepay_stream.wasm`.

## Testing

```bash
cargo test          # unit and integration tests
cargo fmt --check   # formatting
cargo clippy --all-targets   # lints
```

The suite covers the vesting math in isolation and the contract end to end:
stepwise withdrawal, cliff gating, cancellation splits, authorization
requirements, invalid input, and double-withdraw and unknown-id guards.

## Deploying to testnet

`scripts/deploy.sh` wraps the Stellar CLI to build, install, and deploy the
contract. It expects a funded identity configured with `stellar keys`.

```bash
./scripts/deploy.sh <identity-name>
```

## Project structure

```
contracts/stream/src/
  lib.rs        module wiring and public exports
  contract.rs   entry points: create, withdraw, cancel, views
  vesting.rs    pure linear-vesting calculations
  types.rs      Stream record and StreamStatus
  storage.rs    persistent storage keys and TTL handling
  events.rs     Created, Withdrawn, Cancelled events
  error.rs      contract error codes
  test.rs       integration tests and the shared test harness
```

## Related repositories

- **tricklepay-backend** — indexes stream events and serves a read API.
- **tricklepay-frontend** — web client for creating and managing streams.
- **tricklepay-docs** — architecture, security model, and contributor guides.

## License

MIT. See [LICENSE](LICENSE).
