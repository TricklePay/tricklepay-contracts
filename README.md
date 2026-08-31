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

**All timestamps are Unix seconds.** The `start_time`, `end_time`, and `cliff_time` parameters are `u64` Unix timestamps in seconds, matching the Soroban ledger clock (`env.ledger().timestamp()`). A caller using milliseconds (such as JavaScript's `Date.now()`) would create a stream that appears to never start, since a timestamp like `1735689600000` (January 1, 2025 in milliseconds) is interpreted as a date billions of years in the future when read as seconds. The contract does not validate timestamp magnitude or convert units; the caller must ensure all times are in seconds.

**Concrete example:** To create a one-month stream starting on **January 1, 2025 at 00:00:00 UTC** and ending on **February 1, 2025 at 00:00:00 UTC**, convert both dates to Unix seconds:
- January 1, 2025 00:00:00 UTC = `1735689600` seconds since the Unix epoch (not `1735689600000` milliseconds).
- February 1, 2025 00:00:00 UTC = `1738368000` seconds.

Call `create_stream(sender, recipient, token, total_amount, 1735689600, 1738368000, 1735689600)` where `cliff_time == start_time` represents the no-cliff case. The ledger clock increments in seconds, so vesting progresses one second at a time from `start_time` toward `end_time`.

A stream is defined by a total amount and a window of time:

- **Start and end** bound the linear release. At the start nothing has vested;
  at the end the full amount has vested; in between the vested amount grows in
  proportion to elapsed time. The `end_time` must be strictly in the future at
  the moment `create_stream` is called — a window whose end has already passed
  is rejected with `StreamWindowInPast`. A window whose `start_time` is in the
  past but whose `end_time` is still in the future is accepted: the elapsed
  portion vests immediately, making it useful for backdated payroll or grants
  that should have started earlier.
- **Cliff** (optional) is a point before which nothing can be withdrawn. When
  the cliff is reached, everything accrued since the start unlocks at once and
  vesting continues linearly from there. `cliff_time` must fall inside
  `[start_time, end_time]`; anything outside is rejected with `InvalidCliff`.

  **A stream has no cliff when `cliff_time == start_time`.** There is no
  separate flag or null value to pass — the cliff is always a timestamp, and
  setting it to the start makes the gate vacuous. `vested_amount` withholds
  everything while `now < cliff_time || now < start_time`, so when the two are
  equal that reduces to `now < start_time`: exactly the start check every
  stream already applies. The no-cliff case is not special-cased anywhere in
  the vesting math, it simply falls out of the same expression, and from
  `start_time` onward the amount is the plain linear
  `total_amount * elapsed / duration`. At the other end of the range,
  `cliff_time == end_time` is equally valid and withholds everything until the
  window closes — a pure lockup that vests in one step.

  A no-cliff stream is what `create_stream(sender, recipient, token, 1000, 100,
1100, 100)` opens, and it is the shape most of the contract tests use. Its
  schedule is tabulated under [Example schedule](#example-schedule) below.

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
rendering a progress bar (for example, a value of 5000 means 50%). Cancelling
freezes the total at whatever had vested, so a cancelled stream reports nothing
locked and full progress (10000) even when it was stopped early. A stream with
a `total_amount` of zero also reports full progress (10000) at all times.

All amounts are in the token's smallest unit. All times are Unix timestamps in
seconds, matching the ledger clock.

### Example schedule

Both examples stream **1000 units from `start_time = 100` to `end_time = 1100`**
— the reference stream the vesting tests use. Every row below is asserted in
[`vesting.rs`](contracts/stream/src/vesting.rs).

Without a cliff, `cliff_time == start_time == 100` (no cliff):

| Time | Vested | Locked | Description                                                   |
| ---- | ------ | ------ | ------------------------------------------------------------- |
| 50   | 0      | 1000   | before the start, nothing has vested; entire amount is locked |
| 350  | 250    | 750    | a quarter of the window has elapsed                           |
| 600  | 500    | 500    | the midpoint                                                  |
| 850  | 750    | 250    | three quarters                                                |
| 1100 | 1000   | 0      | the end: fully vested; zero locked                            |
| 9999 | 1000   | 0      | past the end, still capped at the total                       |

With a cliff at the midpoint, `cliff_time == 600`:

| Time | Vested | Locked | Description                                                                 |
| ---- | ------ | ------ | --------------------------------------------------------------------------- |
| 300  | 0      | 1000   | past the start, but the cliff has not been reached; all 1000 remains locked |
| 600  | 500    | 500    | the cliff releases everything accrued since the start, unlocking 500        |
| 850  | 750    | 250    | vesting continues linearly from the cliff onward                            |
| 1100 | 1000   | 0      | the end: fully vested                                                       |

The two schedules agree everywhere from the cliff onward. A cliff does not
change the rate or the total, it only withholds the earlier portion and then
releases it in one step.

### Boundary and edge-case notes

A few common edge cases are worth keeping explicit:

- An exact-end withdrawal is valid: once `now >= end_time`, the stream is fully
  vested and `withdraw` can move the remaining balance out in one call.
- A stream with `cliff_time == start_time` is a normal stream with no cliff; the
  vesting logic simply reduces to the standard start-time gate.
- Cancellation is never retroactive. The recipient keeps all vested funds up to
  the cancellation instant, and the sender receives only the remaining unvested
  balance.

### Integer rounding

Vested amounts are computed as:

```
vested = total_amount * elapsed / duration
```

where `elapsed = now - start_time` and `duration = end_time - start_time`. Both
operands are cast to `i128` before the multiplication so the product never
overflows for any amount at or below the `MAX_AMOUNT` cap (`i64::MAX` stroops).

Because this is **integer (truncating) division**, any fractional stroop is
discarded toward zero. The recipient is never credited more than their exact
linear share — the rounding always favours the contract.

**No-cliff example:** a stream of **1000 units over `[100, 1100]`** with
`cliff_time == start_time == 100` (no cliff):

| Time | `elapsed` | Exact share | Vested (truncated) |
| ---- | --------- | ----------- | ------------------ |
| 350  | 250       | 250.0       | 250                |
| 600  | 500       | 500.0       | 500                |
| 850  | 750       | 750.0       | 750                |
| 1100 | 1000      | 1000.0      | 1000               |

The schedule above divides evenly, so truncation has no visible effect. To see
it, consider **10 units over 3 seconds`** queried at `now == 1`:
`10 * 1 / 3 = 3` (not 4). This is explicitly tested in
[`vesting.rs`](contracts/stream/src/vesting.rs) as `integer_division_rounds_down`.

**Limitation:** a stream whose `total_amount` is not a multiple of `duration`
will silently lose at most `duration - 1` stroops to rounding over the stream's
entire life. For example, 10 units over 3 seconds delivers only 9 (3 + 3 + 3)
rather than 10 — the last stroop never vests as a fractional unit and remains
in the contract after the window closes. Callers who require exact delivery
should size `total_amount` to be a multiple of `duration`, or accept the
rounding delta as a known, bounded cost.

**Compatibility note:** the formula and rounding behaviour are part of the
public contract ABI. Any change to the rounding direction would constitute a
breaking change to the on-chain interface.

## Contract interface

**All amounts are integer base units (stroops), not whole tokens.** When calling `create_stream` or `withdraw_amount`, the `total_amount` and `amount` parameters must be denominated in the token's smallest indivisible unit. For Stellar native assets (XLM) and most Stellar Asset Contract (SAC) tokens, that unit is the stroop: one ten-millionth of a whole token (1 token = 10,000,000 stroops). Passing `100` for a seven-decimal token like XLM streams 0.00001 XLM, not 100 XLM.

**Concrete example:** To stream **50 XLM** from Alice to Bob over one month, the caller must pass `total_amount = 500_000_000` (fifty million stroops) to `create_stream`. Similarly, to withdraw **10 XLM** from a stream, the caller must pass `amount = 100_000_000` (one hundred million stroops) to `withdraw_amount`. The contract does not accept or return whole-token amounts; all arithmetic is performed in base units to avoid fractional token handling.

**Why base units:** Soroban tokens use integer arithmetic. Stellar Asset Contract balances are stored as `i128` stroops, and the contract performs all vesting calculations (`vested = total_amount * elapsed / duration`) in that same unit. Using base units everywhere eliminates rounding errors and keeps the interface aligned with the underlying token contract's transfer and balance semantics.

| Function                                                                                         | Caller    | Description                                                               |
| ------------------------------------------------------------------------------------------------ | --------- | ------------------------------------------------------------------------- |
| `create_stream(sender, recipient, token, total_amount, start_time, end_time, cliff_time) -> u64` | sender    | Locks `total_amount` and opens a stream, returning its id.                |
| `withdraw(id) -> i128`                                                                           | recipient | Transfers the vested, unwithdrawn balance to the recipient.               |
| `withdraw_amount(id, amount) -> i128`                                                            | recipient | Transfers exactly `amount`; fails if it exceeds the withdrawable balance. |
| `cancel(id) -> i128`                                                                             | sender    | Refunds the unvested remainder to the sender and freezes the stream.      |
| `get_stream(id) -> Stream`                                                                       | anyone    | Returns the full stream record.                                           |
| `withdrawable(id) -> i128`                                                                       | anyone    | Amount the recipient can withdraw right now.                              |
| `vested(id) -> i128`                                                                             | anyone    | Total vested so far, including what was withdrawn.                        |
| `locked(id) -> i128`                                                                             | anyone    | Amount still unvested; zero once the stream completes or is cancelled.    |
| `progress(id) -> u32`                                                                            | anyone    | Vesting progress in basis points, from `0` to `10000`.                    |
| `status(id) -> StreamStatus`                                                                     | anyone    | `Pending`, `Streaming`, `Completed`, or `Cancelled`.                      |
| `stream_count() -> u64`                                                                          | anyone    | Number of streams created; ids run from 0 upward.                         |

#### Required authorization signatures

The contract enforces Soroban authorization at the call site using
`require_auth()` on the participant whose action is being authorized:

- `create_stream(...)` requires `sender.require_auth()`.
- `withdraw(...)` and `withdraw_amount(...)` require `recipient.require_auth()`.
- `cancel(...)` requires `sender.require_auth()`.

A missing or invalid signature is a host-auth failure, not a `StreamError`.
That is intentional: authorization errors are reported by Soroban before the
contract returns a user-facing enum value.

**Concrete example:** if Alice creates a stream to Bob using token `T`, the
wallet or client must sign the invocation with Alice's key. If the signer is
not Alice, the authorization step fails before the contract can check the
stream schedule or transfer funds.

**Compatibility note:** the contract does not accept a custom “approval token”
for these checks; the required signature mechanism is the standard Soroban
`Address::require_auth()` flow. Client code should therefore attach the exact
caller signature expected by the entry point rather than relying on a
non-standard allowance path.

#### `create_stream` validation order

Arguments are validated in a fixed order, and **all of it runs before any
tokens move or any storage is written** — a rejected call leaves no transfer,
no stream record, and no consumed id behind. When an argument list breaks more
than one rule, the first group below decides the error, so integrators get the
same answer every time rather than one that depends on check ordering:

|     | Group         | Errors, in order                                                   |
| --- | ------------- | ------------------------------------------------------------------ |
| 1   | Authorization | `sender` must authorize the call                                   |
| 2   | Participants  | `InvalidParticipant`                                               |
| 3   | Amount        | `InvalidAmount`, then `AmountTooLarge`                             |
| 4   | Schedule      | `InvalidTimeRange`, then `InvalidCliff`, then `StreamWindowInPast` |
| 5   | Capacity      | `StreamCountExhausted`                                             |

Two participant rules are enforced in group 2. `sender` and `recipient` must
differ, and the token address must also be distinct from both of them. A stream
where `token == sender` or `token == recipient` is invalid because the token
contract cannot also act as a stream participant. The stream contract's own
address is also not valid in any role (`sender`, `recipient`, or `token`),
therefore each one triggers `InvalidParticipant` before any token transfer.

The first four calls move tokens and require authorization from the caller
named above. The rest are read-only views computed from the stream record and
the current ledger time; those that take an id return `StreamNotFound` when no
stream has it.

### Token allowance requirements

When calling `create_stream`, the full `total_amount` of tokens is pulled immediately from the `sender` into the stream contract address via `TokenClient::transfer(&sender, &contract_address, &total_amount)` (see [`contract.rs`](contracts/stream/src/contract.rs#L133-L137)).

- **Allowance Expectation:** The contract expects the `sender` to have a sufficient token balance and to have authorized the token transfer. On Soroban (SEP-41 / Stellar Asset Contract standard), calling `create_stream` invokes `sender.require_auth()`. In client integrations, the `sender` must either include the token transfer in their invocation authorization or grant an allowance to the stream contract equal to or exceeding `total_amount`.
- **How it's checked:** The allowance and balance check occurs in step 5 of `create_stream` after all validation checks (authorization, participants, amount, schedule, capacity) pass. If `sender` lacks sufficient balance or token allowance/authorization, the token transfer panics before any stream state is created or stored.

#### Worked `create_stream` example

Alice wants to stream **1,000 stroops** of token `T` to Bob from Unix
timestamp `100` through timestamp `1100`, with no cliff. Assume a local or test
ledger whose current timestamp is `50`, and that this is the first stream
created by a fresh contract deployment. The complete call is:

```text
stream_id = create_stream(
    Alice,   // sender
    Bob,     // recipient
    token_T, // token contract
    1_000,   // total_amount, in the token's smallest unit
    100,     // start_time, in Unix seconds
    1_100,   // end_time, in Unix seconds
    100,     // cliff_time; equal to start_time means no cliff
)

stream_id == 0
```

Alice must authorize the call and have at least 1,000 stroops available. When
the call succeeds, the full 1,000 stroops are transferred from Alice into the
stream contract immediately; creation does not transfer the tokens gradually.
The contract then stores the funded schedule and returns `0`, the new stream's
`u64` id. Stream ids increase from zero, so the next successful creation returns
`1`. Bob and Alice use the returned id in later calls such as `get_stream(0)`,
`withdraw(0)`, and `cancel(0)`.

For this schedule, vesting follows `vested = total_amount * elapsed / duration`
and matches the no-cliff example in
[`vesting.rs`](contracts/stream/src/vesting.rs#L108-L116).

Verification and test implementations can be reviewed in [`test.rs`](contracts/stream/src/test.rs#L128-L157).

### Error codes

| Code | Variant                  | When returned                                                                                                                               |
| ---- | ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| 1    | `StreamNotFound`         | No stream exists for the given id.                                                                                                          |
| 3    | `InvalidTimeRange`       | `start_time` is not strictly before `end_time`.                                                                                             |
| 4    | `InvalidAmount`          | `total_amount` is zero or negative, or the withdrawal amount is non-positive.                                                               |
| 5    | `InvalidCliff`           | `cliff_time` falls outside `[start_time, end_time]`.                                                                                        |
| 6    | `AlreadyCancelled`       | Attempting to cancel a stream that was already cancelled.                                                                                   |
| 7    | `NothingToWithdraw`      | No vested balance is available to withdraw right now.                                                                                       |
| 8    | `InsufficientBalance`    | Requested withdrawal exceeds the available vested balance.                                                                                  |
| 9    | `StreamAlreadyCompleted` | Attempting to cancel a stream that has fully vested (`now >= end_time`).                                                                    |
| 10   | `AmountTooLarge`         | `total_amount` exceeds `i64::MAX`, the overflow-safety cap.                                                                                 |
| 11   | `StreamWindowInPast`     | `end_time` is at or before the current ledger timestamp. The stream would be 100 % vested on creation; use a direct token transfer instead. |
| 12   | `StreamCountExhausted`   | The id counter has reached `u64::MAX`. No further stream can be created; ids are never reused.                                              |
| 13   | `InvalidParticipant`     | `sender` equals `recipient`, or `sender`/`recipient`/`token` is the stream contract's own address.                                          |

Code 2 is permanently retired and will never be assigned to a new variant.

### Event schemas for indexers

The contract publishes `Created`, `Withdrawn`, and `Cancelled` events, each
carrying the parties as topics so an indexer can filter streams by sender or
recipient. `Created` also carries the schedule, so a stream can be recorded
without a follow-up `get_stream` call, and `withdraw` and `withdraw_amount`
publish the same `Withdrawn` event.

All event definitions are in [`events.rs`](contracts/stream/src/events.rs). Each event uses Soroban's `#[contractevent]` macro and marks certain fields with `#[topic]` to enable efficient indexer filtering by address.

**Created**

```rust
pub struct Created {
    #[topic] sender: Address,
    #[topic] recipient: Address,
    id: u64,
    token: Address,
    total_amount: i128,
    start_time: u64,
    end_time: u64,
    cliff_time: u64,
}
```

Published when `create_stream` succeeds. Both `sender` and `recipient` are indexed topics so an indexer can query all streams for a given address in either role. The schedule fields (`total_amount`, `start_time`, `end_time`, `cliff_time`) allow full stream reconstruction without a follow-up `get_stream` call.

**Withdrawn**

```rust
pub struct Withdrawn {
    #[topic] recipient: Address,
    id: u64,
    amount: i128,
}
```

Published by both `withdraw` and `withdraw_amount` when tokens are transferred to the recipient. `recipient` is a topic for filtering withdrawal activity by address. The `amount` field is the actual number of base units transferred in this withdrawal event.

**Cancelled**

```rust
pub struct Cancelled {
    #[topic] sender: Address,
    id: u64,
    recipient_amount: i128,
    sender_refund: i128,
}
```

Published when `cancel` stops a stream. `sender` is a topic. Both sides of the split are included: `recipient_amount` is the vested portion that remains claimable by the recipient, and `sender_refund` is the unvested amount refunded to the sender.

**Compatibility note:** event field names, types, and topic markers form part of the contract's public ABI. Any change is breaking for indexers and off-chain consumers that depend on the topic layout to filter streams by participant. See the documentation comment in [`events.rs`](contracts/stream/src/events.rs) for the full rationale.

**Concrete example:** an indexer filtering for all streams where Alice is the recipient would subscribe to `Created` events with `recipient == Alice` and to `Withdrawn` events with `recipient == Alice`. The `Created` event carries the full schedule, so the indexer can reconstruct the stream record and track its vesting progress over time. When Alice withdraws, the `Withdrawn` event provides the exact amount transferred without requiring a follow-up query to the contract.

## Stream enumeration

**On-chain enumeration of streams by address is deliberately not supported.**

Streams are keyed by numeric id only. The contract does not maintain per-sender or per-recipient index lists for the following reasons:

- Soroban persistent storage is paid per entry and per ledger. Maintaining a dynamic list of ids under each address key would require unbounded storage growth and complex TTL management, imposing costs on every `create_stream` call that are proportional to how active the address is.
- A contract-side list would need a maximum length cap or pagination scheme, adding surface area for bugs and gas exhaustion attacks.

**How to enumerate streams for an address:**

Use the `Created` event. Each `Created` event is published with `sender` and `recipient` as indexed topics, so any indexer (Horizon, RPC, or the tricklepay-backend) can filter events by topic to reconstruct the full set of stream ids for any address without a follow-up `get_stream` call. The event also carries the full schedule, so streams can be recorded on first observation.

For a contract-only consumer with no event access:

1. Call `stream_count()` to get the total number of streams.
2. Call `get_stream(id)` for each id from `0` to `stream_count() - 1` and filter by `sender` or `recipient`.

This is O(n) over all streams and is only suitable for small deployments or one-off queries. Production consumers should use event indexing.

## Storage lifetime

Soroban storage entries expire on a ledger clock and are archived once their
time to live (TTL) runs out. The contract keeps two kinds of entry alive on the same
schedule:

| Entry                  | Storage type | Holds                               |
| ---------------------- | ------------ | ----------------------------------- |
| `DataKey::Stream(id)`  | persistent   | one stream record                   |
| `DataKey::StreamCount` | instance     | the id to assign to the next stream |

Both are granted `ENTRY_TTL` — 518,400 ledgers, roughly thirty days at the
standard five second close time — and both are extended back to that full
window whenever they are touched with fewer than `BUMP_THRESHOLD` (103,680
ledgers, roughly six days) remaining. Above that mark a touch is a deliberate
no-op, so an entry in frequent use does not pay to be re-extended on every
access.

The two are refreshed by different things:

- **Stream entries** are bumped as a side effect of being read or written, so
  `get_stream`, `withdrawable`, `vested`, `locked`, `progress`, `status`,
  `withdraw`, `withdraw_amount`, and `cancel` all renew the stream they touch.
  A stream that is looked at even once every few weeks never expires, and a
  stream may run far longer than a single `ENTRY_TTL` window.
- **The instance** is bumped only by `create_stream`. Nothing renews it as a
  side effect of a read, so a contract that is queried but never written to
  will run its instance down.

That second point is why `create_stream` extends the instance explicitly. The
counter is the source of every id and never reuses one; if the instance were
archived, a fresh counter would restart at zero and the next stream would be
written over the record still sitting under `Stream(0)`. Extending the instance
on the same schedule as stream entries keeps the counter alive for as long as
the streams it numbers.

### TTL and archival behavior

**Time-to-live mechanics:** Every stream entry and the contract instance has a TTL counter that decrements by one with each closed ledger. When the TTL reaches zero, the entry is archived and removed from active storage. Archived entries are no longer accessible via contract calls and must be restored through an off-chain Soroban restore operation before they can be read or modified again.

**Automatic extension:** The contract extends TTL automatically when an entry is accessed and its remaining TTL is below `BUMP_THRESHOLD` (103,680 ledgers, roughly six days). The extension resets the TTL back to the full `ENTRY_TTL` window (518,400 ledgers, roughly thirty days). If the remaining TTL is above the threshold, no extension occurs to avoid paying unnecessary storage fees on every access.

**What triggers extension:**
- Any call to `get_stream`, `withdrawable`, `vested`, `locked`, `progress`, `status`, `withdraw`, `withdraw_amount`, or `cancel` for a stream id extends that stream's TTL if it is below the bump threshold.
- `create_stream` extends the instance TTL, ensuring the id counter remains accessible.

**Archival limitation:** A stream left completely untouched for longer than `ENTRY_TTL` (518,400 ledgers, roughly thirty days) is archived. Once archived:
- The stream cannot be accessed via contract calls. Attempts to call `get_stream`, `withdraw`, or any other function for that stream id return a host storage error, not a `StreamError::StreamNotFound`.
- The contract itself offers no way to revive an archived stream. Recovery requires an off-chain Soroban restore transaction submitted directly to the network.
- Tokens locked in an archived stream remain in the contract address until the entry is restored and the stream is interacted with again.

**Concrete example:** Alice creates a six-month stream to Bob on January 1. Bob does not check or withdraw from the stream for the entire six months. On February 1 (roughly 518,400 ledgers later, assuming a five-second ledger close time), the stream entry's TTL reaches zero and is archived. On July 1, when the stream has fully vested, Bob attempts to call `withdraw`. The call fails because the stream entry is archived. Bob must submit a Soroban restore transaction to bring the entry back into active storage, after which `withdraw` will succeed and transfer the vested tokens.

**How to avoid archival:** Callers holding long-dated streams should read them periodically — any view call is enough. A single `get_stream(id)` or `withdrawable(id)` call every few weeks (well within the 30-day window) keeps the stream alive indefinitely. Indexers that track streams by listening to `Created` events can implement automated TTL extension by periodically querying tracked stream ids.

**Compatibility note:** The TTL values (`ENTRY_TTL = 518_400` and `BUMP_THRESHOLD = 103_680`) are defined in the contract source ([`contract.rs`](contracts/stream/src/contract.rs)) and form part of the operational behavior. Changing these values in a redeployed contract would alter the archival window, affecting how often streams must be accessed to stay alive. The archival mechanism itself is part of the Soroban platform and cannot be disabled at the contract level.

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

**Minimum Supported Rust Version (MSRV):** The MSRV is `1.84.0`. This recent toolchain is required because `soroban-sdk` targets `wasm32v1-none`. The pinned versions are in `rust-toolchain.toml`.

Note that `wasm32-unknown-unknown` does not work: on Rust 1.82+ it enables wasm features the Soroban environment does
not support, and soroban-sdk fails the build rather than produce a bad artifact.

```bash
# Native build and the full test suite
cargo test

# Optimized WASM ready to deploy
cargo build --release --target wasm32v1-none
```

The release artifact is written to
`target/wasm32v1-none/release/tricklepay_stream.wasm`.

### Release profile

Contract size affects deployment cost, since Soroban charges to store and load
bytecode. The `[profile.release]` settings in the root `Cargo.toml` are tuned
to minimize the size of that artifact, sometimes at the cost of build time or
raw runtime speed:

| Setting            | Value       | Effect                                                                                                                                                                                                           |
| ------------------ | ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `opt-level`        | `"z"`       | Optimizes for the smallest possible binary size, ahead of runtime speed.                                                                                                                                         |
| `lto`              | `true`      | Enables link-time optimization across the whole dependency graph, so the compiler can inline and eliminate dead code across crate boundaries — smaller (and often faster) output, at the cost of a slower build. |
| `codegen-units`    | `1`         | Compiles as a single codegen unit instead of splitting work in parallel, which allows more aggressive cross-function optimization at the cost of slower, non-parallel compilation.                               |
| `strip`            | `"symbols"` | Strips debug symbols and other metadata from the compiled artifact, shrinking it with no effect on behavior.                                                                                                     |
| `debug`            | `0`         | Omits debug info from the build; it isn't used in a deployed WASM artifact and only adds size.                                                                                                                   |
| `debug-assertions` | `false`     | Disables `debug_assert!` checks in the compiled output, trimming both size and runtime overhead.                                                                                                                 |
| `overflow-checks`  | `true`      | Kept enabled even in release mode, unlike the Rust default — a deliberate trade of a small amount of size and speed for safety, since a silent overflow in a token amount would be a serious bug.                |
| `panic`            | `"abort"`   | Aborts on panic instead of unwinding, removing the unwinding machinery from the binary for a smaller artifact.                                                                                                   |

## Testing

```bash
cargo test          # unit and integration tests
cargo fmt --check   # formatting
cargo clippy --all-targets   # lints
cargo audit --deny warnings   # uses .cargo/audit.toml ignores
```

Before opening a pull request, run:

```bash
make check
```

This runs formatting, linting, and the test suite in the same order as CI, and
fails fast on the first error.

The audit ignores the unmaintained `derivative` and `paste` crates
(`RUSTSEC-2024-0388` and `RUSTSEC-2024-0436`) and the yanked `spin` crate via
`.cargo/audit.toml` because they are transitive Soroban test-host dependencies
and are not used in the deployed WASM. Vulnerability advisories remain enabled;
see `.cargo/audit.toml` for the allowlist.

The suite covers the vesting math in isolation and the contract end to end:
stepwise withdrawal, partial withdrawal and its over-request and non-positive
guards, cliff gating, cancellation splits, the `locked` and `progress` views
across a stream's life, the cliff and no-cliff schedules documented above,
authorization requirements, invalid input, past and
boundary time-window rejection, backdated-start acceptance, multiple token
parallel streams, id-counter exhaustion at the `u64::MAX` boundary, rejection
of the contract's own address in each participant role, self-streams, the
documented precedence between validation groups, and double-withdraw and unknown-id guards.

It also covers the storage and event behaviour described above: the order in
which each entry point moves tokens and publishes its event, the indexed
event topics, the silence of a rejected call on the event stream, `DataKey`
encoding across the id range, and
the persistent-entry and instance time-to-live bumps on both sides of
`BUMP_THRESHOLD`.

## Deploying to testnet

`scripts/deploy.sh` wraps the Stellar CLI to build, install, and deploy the
contract. It expects a funded identity configured with `stellar keys`.

```bash
./scripts/deploy.sh <identity-name>
```

## Troubleshooting

### Wrong build target
If you see an error when building that mentions unsupported WebAssembly features or the wrong target:
```text
error: compiling for `wasm32-unknown-unknown` is not supported
```
**Fix:** Soroban SDK requires the newer target on recent Rust versions. Always build with `--target wasm32v1-none` instead of `wasm32-unknown-unknown`.

### Missing toolchain component
If you see an error indicating that the standard library cannot be found:
```text
error[E0463]: can't find crate for `core`
  = note: the `wasm32v1-none` target may not be installed
```
**Fix:** Add the required WebAssembly target to your Rust toolchain by running:
`rustup target add wasm32v1-none`

### Unfunded identity
When running the deployment script, if you encounter an error like:
```text
error: account not found
```
or a transaction failure due to insufficient XLM on testnet.
**Fix:** Make sure the identity you are using is funded by running:
`stellar keys fund <identity-name> --network testnet`

## Frequently asked questions

**1. Why are funds locked up front?**
To guarantee that the recipient will actually receive the streamed tokens, the entire `total_amount` is pulled into the contract immediately upon creation. This prevents the sender from spending the funds elsewhere before they vest. See [THREAT_MODEL.md](THREAT_MODEL.md) for details on the security implications of this lock-up.

**2. What happens if the project's servers disappear?**
The stream lives entirely on the Stellar ledger as a smart contract. You can interact with it using any Stellar Horizon or RPC node, even if our frontend or indexer goes down. The security model ensures that you do not depend on any off-chain infrastructure. See [THREAT_MODEL.md](THREAT_MODEL.md).

**3. Can I pause or freeze a stream?**
No. There is no pause, freeze, or emergency-stop function. The only escape hatch is the sender's `cancel` function, which stops the stream and refunds only the unvested portion. See [THREAT_MODEL.md](THREAT_MODEL.md).

**4. Are there any admin keys that can steal or lock my funds?**
No, there is no admin or owner account. The deployed bytecode is immutable, meaning no privileged key can upgrade the contract, halt streams, or confiscate tokens. See [THREAT_MODEL.md](THREAT_MODEL.md).

**5. How do I enumerate my streams?**
On-chain enumeration is not supported to save on storage and gas costs. You should use the `Created` event to index streams off-chain. See the Stream enumeration section above for more details.

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
