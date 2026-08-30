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
rendering a progress bar. Cancelling freezes the total at whatever had vested,
so a cancelled stream reports nothing locked and full progress even when it was
stopped early.

All amounts are in the token's smallest unit. All times are Unix timestamps in
seconds, matching the ledger clock.

### Example schedule

Both examples stream **1000 units from `start_time = 100` to `end_time = 1100`**
— the reference stream the vesting tests use. Every row below is asserted in
[`vesting.rs`](contracts/stream/src/vesting.rs).

Without a cliff, `cliff_time == start_time == 100` (no cliff):

| Time | Vested | Locked | Description |
| --- | --- | --- | --- |
| 50 | 0 | 1000 | before the start, nothing has vested; entire amount is locked |
| 350 | 250 | 750 | a quarter of the window has elapsed |
| 600 | 500 | 500 | the midpoint |
| 850 | 750 | 250 | three quarters |
| 1100 | 1000 | 0 | the end: fully vested; zero locked |
| 9999 | 1000 | 0 | past the end, still capped at the total |

With a cliff at the midpoint, `cliff_time == 600`:

| Time | Vested | Locked | Description |
| --- | --- | --- | --- |
| 300 | 0 | 1000 | past the start, but the cliff has not been reached; all 1000 remains locked |
| 600 | 500 | 500 | the cliff releases everything accrued since the start, unlocking 500 |
| 850 | 750 | 250 | vesting continues linearly from the cliff onward |
| 1100 | 1000 | 0 | the end: fully vested |

The two schedules agree everywhere from the cliff onward. A cliff does not
change the rate or the total, it only withholds the earlier portion and then
releases it in one step.

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
| --- | --- | --- | --- |
| 350 | 250 | 250.0 | 250 |
| 600 | 500 | 500.0 | 500 |
| 850 | 750 | 750.0 | 750 |
| 1100 | 1000 | 1000.0 | 1000 |

The schedule above divides evenly, so truncation has no visible effect. To see
it, consider **10 units over `[0, 3]`** queried at `now == 1`:
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

| Function | Caller | Description |
| --- | --- | --- |
| `create_stream(sender, recipient, token, total_amount, start_time, end_time, cliff_time) -> u64` | sender | Locks `total_amount` and opens a stream, returning its id. |
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

| | Group | Errors, in order |
| --- | --- | --- |
| 1 | Authorization | `sender` must authorize the call |
| 2 | Participants | `InvalidParticipant` |
| 3 | Amount | `InvalidAmount`, then `AmountTooLarge` |
| 4 | Schedule | `InvalidTimeRange`, then `InvalidCliff`, then `StreamWindowInPast` |
| 5 | Capacity | `StreamCountExhausted` |

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
- **Worked example:**
  1. A sender holds **1,000 stroops** of token `T`.
  2. The sender approves/authorizes the stream contract to transfer **1,000 stroops** of token `T`.
  3. The sender invokes `create_stream(sender, recipient, token_T, 1000, 100, 1100, 100)` (where `cliff_time == start_time == 100` represents the no-cliff vesting case).
  4. Step 5 executes `TokenClient::new(&env, &token_T).transfer(&sender, &contract_address, &1000)`.
  5. The contract balance increases by 1,000 stroops, the sender balance decreases by 1,000 stroops, and stream ID `0` is initialized with linear vesting math `vested = total_amount * elapsed / duration` matching the no-cliff example schedule in [`vesting.rs`](contracts/stream/src/vesting.rs#L108-L116).

Verification and test implementations can be reviewed in [`test.rs`](contracts/stream/src/test.rs#L128-L157).

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
| 11 | `StreamWindowInPast` | `end_time` is at or before the current ledger timestamp. The stream would be 100 % vested on creation; use a direct token transfer instead. |
| 12 | `StreamCountExhausted` | The id counter has reached `u64::MAX`. No further stream can be created; ids are never reused. |
| 13 | `InvalidParticipant` | `sender` equals `recipient`, or `sender`/`recipient`/`token` is the stream contract's own address. |

Code 2 is permanently retired and will never be assigned to a new variant.

The contract publishes `Created`, `Withdrawn`, and `Cancelled` events, each
carrying the parties as topics so an indexer can filter streams by sender or
recipient. `Created` also carries the schedule, so a stream can be recorded
without a follow-up `get_stream` call, and `withdraw` and `withdraw_amount`
publish the same `Withdrawn` event.

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
time to live runs out. The contract keeps two kinds of entry alive on the same
schedule:

| Entry | Storage type | Holds |
| --- | --- | --- |
| `DataKey::Stream(id)` | persistent | one stream record |
| `DataKey::StreamCount` | instance | the id to assign to the next stream |

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

**Limitation:** a stream left completely untouched for longer than `ENTRY_TTL`
is archived like any other Soroban entry. Recovering it requires a restore
operation submitted off-contract; the contract itself offers no way to revive
an archived stream. Callers holding long-dated streams should read them
periodically — any view call is enough.

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
cargo audit --deny warnings   # uses .cargo/audit.toml ignores
```

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
boundary time-window rejection, backdated-start acceptance, multiple
recipients funded by one sender, id-counter exhaustion at the `u64::MAX`
boundary, rejection of the contract's own address in each participant role,
self-streams, the documented precedence between validation groups, and
double-withdraw and unknown-id guards.

It also covers the storage and event behaviour described above: the order in
which each entry point moves tokens and publishes its event, the Created,
Withdrawn, and Cancelled payload fields, the silence of a rejected call on
the event stream, `DataKey` encoding across the id range, and
the persistent-entry and instance time-to-live bumps on both sides of
`BUMP_THRESHOLD`.

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
