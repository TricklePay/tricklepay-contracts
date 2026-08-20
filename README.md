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
  already taken. A partial withdrawal (`withdraw_amount`) names a figure
  instead and transfers exactly that, up to the same balance; whatever is left
  stays in the stream and keeps growing as more vests. The two can be mixed
  freely — draw a fixed sum each month, then sweep the remainder at the end.
- **Cancel** stops a stream early. The recipient keeps everything vested up to
  that moment; the unvested remainder is refunded to the sender. A cancelled
  stream's vested balance stays claimable.

A stream can also be read at any time without changing it. The vested and
`locked` amounts mirror each other and always sum to the total, while
`progress` reports the same ratio in basis points, from 0 to 10000, for
rendering a progress bar. Cancelling freezes the total at whatever had vested,
so a cancelled stream reports nothing locked and full progress even when it was
stopped early.

All amounts are in the token's smallest unit. All times are Unix timestamps in
seconds, matching the ledger clock.

## Contract interface

| Function | Caller | Description |
| --- | --- | --- |
| `create_stream(sender, recipient, token, total_amount, start_time, end_time, cliff_time) -> u64` | sender | Locks `total_amount` and opens a stream, returning its id. `sender` and `recipient` must be different addresses. |
| `withdraw(id) -> i128` | recipient | Transfers the vested, unwithdrawn balance to the recipient. |
| `withdraw_amount(id, amount) -> i128` | recipient | Transfers exactly `amount`; fails if it exceeds the withdrawable balance. |
| `cancel(id) -> i128` | sender | Refunds the unvested remainder to the sender and freezes the stream. |
| `get_stream(id) -> Stream` | anyone | Returns the full stream record. |
| `withdrawable(id) -> i128` | anyone | Amount the recipient can withdraw right now. |
| `vested(id) -> i128` | anyone | Total vested so far, including what was withdrawn. |
| `locked(id) -> i128` | anyone | Amount still unvested; zero once the stream completes or is cancelled. |
| `progress(id) -> u32` | anyone | Vesting progress in basis points, from `0` to `10000`. |
| `status(id) -> StreamStatus` | anyone | `Pending`, `Streaming`, `Completed`, or `Cancelled`. |
| `stream_count() -> u64` | anyone | Number of streams created; ids run from 0 upward. |

The first four calls move tokens and require authorization from the caller
named above. The rest are read-only views computed from the stream record and
the current ledger time; those that take an id return `StreamNotFound` when no
stream has it.

### Error codes

| Code | Variant | When returned |
| --- | --- | --- |
| 1 | `StreamNotFound` | No stream exists for the given id. |
| 3 | `InvalidTimeRange` | `start_time` is not strictly before `end_time`. |
| 4 | `InvalidAmount` | `total_amount` is zero or negative, or the withdrawal amount is non-positive. |
| 5 | `InvalidCliff` | `cliff_time` falls outside `[start_time, end_time]`. |
| 6 | `AlreadyCancelled` | Attempting to cancel a stream that was already cancelled. |
| 7 | `NothingToWithdraw` | No vested balance is available to withdraw right now. |
| 8 | `InsufficientBalance` | Requested withdrawal exceeds the available vested balance. |
| 9 | `StreamAlreadyCompleted` | Attempting to cancel a stream that has fully vested (`now >= end_time`). |
| 10 | `AmountTooLarge` | `total_amount` exceeds `i64::MAX`, the overflow-safety cap. |
| 11 | `SenderIsRecipient` | `sender` and `recipient` are the same address. Streaming to yourself is a no-op that wastes fees; use a direct token transfer instead. |

Code 2 is permanently retired and will never be assigned to a new variant.

The contract publishes `Created`, `Withdrawn`, and `Cancelled` events, each
carrying the parties as topics so an indexer can filter streams by sender or
recipient. `Created` also carries the schedule, so a stream can be recorded
without a follow-up `get_stream` call, and `withdraw` and `withdraw_amount`
publish the same `Withdrawn` event.

## Security model

**The contract has no pause, freeze, or emergency-stop function.** There is no
admin or owner account. The deployed bytecode is immutable — there is no
upgrade path. If a bug is discovered after deployment, in-flight streams
cannot be halted or migrated; every token locked in a stream is exposed to any
vulnerability in the deployed code for the full duration of that stream.

The only unilateral escape hatch available to either party is the sender's
`cancel`, which returns the unvested portion to the sender. It does not recover
tokens that have already vested.

This is an explicit design choice: adding a pause mechanism would introduce a
privileged key whose compromise could freeze every stream on the contract
simultaneously. The design removes that risk at the cost of operational
flexibility.

Full details — including the rationale, consequences for lock-up decisions, and
out-of-scope risks — are in [THREAT_MODEL.md](THREAT_MODEL.md).

## Building

Rust 1.84 or newer with the `wasm32v1-none` target is required; the pinned
versions are in `rust-toolchain.toml`. Note that `wasm32-unknown-unknown` does
not work: on Rust 1.82+ it enables wasm features the Soroban environment does
not support, and soroban-sdk fails the build rather than produce a bad artifact.

```bash
# Native build and the full test suite
cargo test

# Optimized WASM ready to deploy
cargo build --release --target wasm32v1-none
```

The release artifact is written to
`target/wasm32v1-none/release/tricklepay_stream.wasm`.

## Testing

```bash
cargo test          # unit and integration tests
cargo fmt --check   # formatting
cargo clippy --all-targets   # lints
```

The suite covers the vesting math in isolation and the contract end to end:
stepwise withdrawal, partial withdrawal and its over-request and non-positive
guards, cliff gating, cancellation splits, the `locked` and `progress` views
across a stream's life, authorization requirements, invalid input, self-stream
rejection, and double-withdraw and unknown-id guards.

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

## Changelog

Notable changes are recorded in [CHANGELOG.md](CHANGELOG.md), including
changes to the error codes, which are part of the public ABI.

The security properties and known limitations described above are documented
in full in [THREAT_MODEL.md](THREAT_MODEL.md).

## Related repositories

- **tricklepay-backend** — indexes stream events and serves a read API.
- **tricklepay-frontend** — web client for creating and managing streams.
- **tricklepay-docs** — architecture, security model, and contributor guides.

## License

MIT. See [LICENSE](LICENSE).
