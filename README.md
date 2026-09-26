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
  the vesting math, it simply falls out of the same expression. This is the usual default when a stream should begin vesting immediately from `start_time` rather than waiting for an explicit cliff. At the other end of the range,
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
| ---- | ------ | ------ | ----------------------------------------------------------------------------- |
| 300  | 0      | 1000   | past the start, but the cliff has not been reached; all 1000 remains locked |
| 600  | 500    | 500    | the cliff releases everything accrued since the start, unlocking 500        |
| 850  | 750    | 250    | vesting continues linearly from the cliff onward                            |
| 1100 | 1000   | 0      | the end: fully vested                                                       |

The two schedules agree everywhere from the cliff onward. A cliff does not
change the rate or the total, it only withholds the earlier portion and then
releases it in one step.

### Worked example: `withdraw_amount`

`withdraw_amount` is easy to confuse with `withdraw`: both pay out vested
tokens, but `withdraw` always sweeps the full available balance while
`withdraw_amount` lets the recipient take a smaller, named amount and leave
the rest streaming.

Using the same no-cliff reference stream from [Example schedule](#example-schedule)
- 1000 units, `start_time = 100`, `end_time = 1100` - at `now = 600` the
midpoint has been reached, so 500 units have vested and none have been
withdrawn yet:

    withdrawable(id) == 500

The recipient draws only 200 of it:

    withdraw_amount(id, 200) -> 200

This transfers exactly 200 units and leaves the remaining 300 of the vested
500 in the stream, still claimable and still separate from whatever vests
next:

    withdrawable(id) == 300

Requesting more than that remaining balance fails outright - nothing is
transferred and nothing is recorded as withdrawn:

    withdraw_amount(id, 400) -> Err(InsufficientBalance)

The call only checks the current withdrawable balance (300), not the
stream's total or its still-locked portion (500), so lowering the request to
300 or less succeeds; asking for 301 or more repeats the same failure until
more of the stream vests.

### Worked example: `cancel`

Using the same no-cliff reference stream — 1000 units, `start_time = 100`,
`end_time = 1100` — cancelled at `now = 600` (the midpoint):

- **500 units have vested.** The recipient's accrued share is frozen and stays
  claimable via `withdraw` or `withdraw_amount` at any time after cancellation.
- **500 units have not vested.** This unvested remainder is refunded to the
  sender immediately by the `cancel` call itself — no separate step required.
- **No further vesting occurs.** The stream is frozen at `total_amount = 500`
  and `end_time = 600`; the vesting window is closed, so the vested amount
  cannot grow beyond what had accrued at the cancellation instant.

```text
cancel(id) -> 500   // 500 refunded to sender; 500 stays claimable by recipient
```

If the recipient had already withdrawn 200 of the 500 vested units before
cancellation, the split is the same — the sender still gets only the 500
unvested units back, not the 200 the recipient already took. The recipient
can then claim the remaining 300 of their vested share:

```text
// at now = 600, after recipient withdrew 200 earlier:
cancel(id)           -> 500   // sender refund (unvested only)
withdraw(id)         -> 300   // recipient claims their remaining vested balance
```
### Worked example: vesting schedule with a cliff

This example uses a one-year stream with a three-month cliff — the shape
typical for employee equity grants. Concrete dates and amounts are used so the
step-change at the cliff is easy to see.

**Parameters:**

| Field | Value | Unix seconds |
| ----- | ----- | ------------ |
| `total_amount` | 12 000 units | — |
| `start_time` | 1 Jan 2025 00:00 UTC | `1735689600` |
| `cliff_time` | 1 Apr 2025 00:00 UTC | `1743465600` |
| `end_time` | 1 Jan 2026 00:00 UTC | `1767225600` |

Duration = 365 days = 31 536 000 seconds.  
Cliff offset from start = 90 days = 7 776 000 seconds.

**Create the stream** (no cliff is expressed as `cliff_time == start_time`; here
we set an explicit cliff):

```text
create_stream(
  sender, recipient, token,
  total_amount = 12000,
  start_time   = 1735689600,   // 1 Jan 2025
  end_time     = 1767225600,   // 1 Jan 2026
  cliff_time   = 1743465600    // 1 Apr 2025
)
```

**Withdrawable amount at three points in time:**

*Before the cliff — 1 Feb 2025 (`now = 1738368000`):*

```
elapsed = 1738368000 - 1735689600 = 2678400 s  (31 days)
now < cliff_time  →  vested = 0
withdrawable = 0
```

One month has passed since the start and 1/12 of the total has accrued by the
linear schedule, but the cliff gate is still blocking it. Nothing can be
withdrawn yet.

*At the cliff — 1 Apr 2025 (`now = 1743465600`):*

```
elapsed = 1743465600 - 1735689600 = 7776000 s  (90 days)
vested  = 12000 * 7776000 / 31536000 = 2958 units  (truncated)
withdrawable = 2958
```

The cliff releases the entire 90 days of accrual in one step. The recipient
can withdraw up to 2958 units immediately, even though nothing was available
one second earlier.

*Six months in — 1 Jul 2025 (`now = 1751328000`):*

```
elapsed = 1751328000 - 1735689600 = 15638400 s  (181 days)
vested  = 12000 * 15638400 / 31536000 = 5950 units  (truncated)
withdrawable = 5950   // assuming nothing withdrawn yet
```

Vesting has continued linearly from the cliff. If the recipient withdrew the
2958 cliff lump on 1 Apr, the withdrawable balance at this point is
`5950 - 2958 = 2992` units.

**Summary table:**

| Date | `now` | Vested | Withdrawable (nothing taken yet) |
| ---- | ----- | ------ | -------------------------------- |
| 1 Feb 2025 (1 month in, before cliff) | `1738368000` | 0 | **0** |
| 1 Apr 2025 (cliff, 3 months in) | `1743465600` | 2958 | **2958** |
| 1 Jul 2025 (6 months in) | `1751328000` | 5950 | **5950** |
| 1 Jan 2026 (end) | `1767225600` | 12000 | **12000** |

The cliff does not change the rate or the total — it only withholds the first
90 days of accrual and releases it all at once when `now` reaches `cliff_time`.
From the cliff onward, the schedule is identical to a no-cliff stream of the
same parameters.

### Worked example: `cancel` with a cliff

When a stream has a cliff, the vested amount before the cliff is **zero**,
even if time has passed since `start_time`. Cancelling before the cliff
refunds the **entire** total to the sender and leaves the recipient with
nothing claimable.

Using a reference stream with a cliff — 1000 units, `start_time = 100`,
`end_time = 1100`, `cliff_time = 600`, cancelled at `now = 300`
(before the cliff):

- **0 units have vested.** The cliff blocks all accrual until `now >= 600`.
- **1000 units are refunded to the sender.** The entire total is unvested.
- **The recipient claimable balance is 0.** Nothing has vested, so nothing
  can be withdrawn.

Cancelling **at** the cliff (or any point after) behaves like the no-cliff
example: whatever has accrued is split between the two parties. At
`now = 600` (the cliff instant), 500 units have vested:

In all cases, cancellation permanently freezes the stream. No further
vesting occurs after the call.

### Fully withdrawn stream: what each view reports

A stream that has been drawn down completely — every token taken out by the
recipient — still exists in storage and answers every view. This is the
expected end state for a healthy stream that ran to completion.

Using the no-cliff reference stream — 1000 units, `start_time = 100`,
`end_time = 1100` — at `now = 1100` (the end), after the recipient has called
`withdraw` and received all 1000 units:

| View | Return value | Reason |
| ---- | ------------ | ------ |
| `get_stream` | stream record with `withdrawn = 1000`, `cancelled = false` | the record is never deleted |
| `withdrawable(id)` | `0` | `vested(1000) - withdrawn(1000) = 0` |
| `vested(id)` | `1000` | at or after `end_time`, the full amount has vested |
| `locked(id)` | `0` | `total_amount(1000) - vested(1000) = 0` |
| `progress(id)` | `10000` | 100 % — fully vested |
| `status(id)` | `Completed` | `now >= end_time` and the stream was not cancelled |

Calling `withdraw` again after the balance is zero returns
`Err(NothingToWithdraw)` — nothing is transferred and nothing is recorded.

**How to tell a finished stream from a broken one.** A normally completed
stream has `status = Completed`, `locked = 0`, `progress = 10000`, and
`withdrawable = 0`. An incomplete or misconfigured stream would show a
non-zero `withdrawable` or `locked` alongside the same `Completed` status,
which means tokens remain unclaimed. The `get_stream` view exposes the raw
`withdrawn` and `total_amount` fields for a precise accounting check:
`withdrawn == total_amount` confirms the recipient has taken everything.

## Events

Every mutating entry point publishes a Soroban event when it succeeds. Rejected
calls publish nothing. Indexers and off-chain consumers use these events to
track stream state without follow-up `get_stream` calls, and to filter streams
by participant address.

**Field order is part of the interface.** The Soroban event encoding preserves
declaration order, so reordering fields in any event struct is a breaking change
for downstream consumers — treat it with the same care as renaming a field or
changing its type.

Each event has a set of *topics* (marked `#[topic]`) that the network indexes
for efficient filtering, and a *data* payload containing the remaining fields.
Topics appear first in the encoding; the data fields follow in declaration order.

### `Created`

Emitted by `create_stream` when a new stream is opened successfully.

| Field | Kind | Type | Description |
|-------|------|------|-------------|
| `sender` | topic | `Address` | The address that funded the stream |
| `recipient` | topic | `Address` | The address that will receive the vested tokens |
| `id` | data | `u64` | The id assigned to the new stream |
| `token` | data | `Address` | The token contract address |
| `total_amount` | data | `i128` | Total tokens locked into the stream |
| `start_time` | data | `u64` | Unix timestamp (seconds) when vesting begins |
| `end_time` | data | `u64` | Unix timestamp (seconds) when the stream is fully vested |
| `cliff_time` | data | `u64` | Unix timestamp (seconds) before which nothing can be withdrawn; equals `start_time` when there is no cliff |

Indexers can subscribe on the `sender` or `recipient` topics to receive all
streams for a given address without scanning every event.

### `Withdrawn`

Emitted by both `withdraw` and `withdraw_amount` when tokens are transferred to
the recipient.

| Field | Kind | Type | Description |
|-------|------|------|-------------|
| `recipient` | topic | `Address` | The address that received the tokens |
| `id` | data | `u64` | The stream that was drawn from |
| `amount` | data | `i128` | Number of tokens transferred in this call |

### `Cancelled`

Emitted by `cancel` when a sender stops a stream early. Both sides of the split
are included so an indexer can record the final state without a follow-up
`get_stream` call.

| Field | Kind | Type | Description |
|-------|------|------|-------------|
| `sender` | topic | `Address` | The address that cancelled the stream and received the refund |
| `id` | data | `u64` | The stream that was cancelled |
| `recipient_amount` | data | `i128` | Vested tokens still claimable by the recipient after cancellation |
| `sender_refund` | data | `i128` | Unvested tokens immediately refunded to the sender |

Note that `recipient_amount` reflects the *remaining claimable balance* at
cancellation time (vested minus already withdrawn), not the total that had
vested. The sender refund covers only the unvested portion; tokens the recipient
had already withdrawn are not returned.

## Reading the contract interface

The contract's public interface — every entry point name, its parameter names
and types, and its return type — can be printed directly from a compiled WASM
artifact without reading the Rust source. This is the canonical way for a
client author to discover the exact call signatures, and it is useful any time
you want to confirm that a deployed binary exposes the interface you expect.

### When to regenerate

- **Before integrating:** read the interface from the artifact you are about to
  deploy so your client code matches the real signatures, not a stale copy.
- **After a code change:** regenerate to confirm that your change added,
  removed, or renamed an entry point as intended.
- **When auditing a deployment:** read the interface from the on-chain WASM to
  check what the live contract actually exposes.

### From a local build

Build the optimised artifact first (the toolchain is pinned in
`rust-toolchain.toml`):

```bash
cargo build --release --target wasm32v1-none
```

Then print the interface with the Stellar CLI:

```bash
stellar contract inspect \
  --wasm target/wasm32v1-none/release/tricklepay_stream.wasm
```

The command reads the custom section that the Soroban SDK embeds in every WASM
at compile time and prints each entry point in an XDR-derived text format.
Output looks like:

```text
fn create_stream(sender: address, recipient: address, token: address,
    total_amount: i128, start_time: u64, end_time: u64, cliff_time: u64)
    -> result<u64, error<contract>>
fn withdraw(id: u64) -> result<i128, error<contract>>
fn withdraw_amount(id: u64, amount: i128) -> result<i128, error<contract>>
fn cancel(id: u64) -> result<i128, error<contract>>
fn get_stream(id: u64) -> result<stream, error<contract>>
fn withdrawable(id: u64) -> result<i128, error<contract>>
fn vested(id: u64) -> result<i128, error<contract>>
fn locked(id: u64) -> result<i128, error<contract>>
fn progress(id: u64) -> result<u32, error<contract>>
fn status(id: u64) -> result<stream_status, error<contract>>
fn stream_count() -> u64
```

### From a deployed contract

Fetch the WASM from the network and inspect it in one step:

```bash
# Replace <CONTRACT_ID> with the deployed bech32 contract address and
# <NETWORK> with "testnet", "mainnet", or a custom RPC URL.
stellar contract fetch \
  --id <CONTRACT_ID> \
  --network <NETWORK> \
  --out-file fetched.wasm

stellar contract inspect --wasm fetched.wasm
```

Or pass `--id` directly to `inspect` if you only need the interface and do not
want to keep the WASM file locally:

```bash
stellar contract inspect \
  --id <CONTRACT_ID> \
  --network <NETWORK>
```

The `stellar` binary used here is the [Stellar CLI](https://github.com/stellar/stellar-cli).
The pinned toolchain in `rust-toolchain.toml` ensures the local artifact
matches the one used during deployment when built on the same platform.

## Verifying a deployment

Anyone can confirm that a live contract was built from this source by comparing
the on-chain bytecode hash with the hash produced by a local build.

### Step 1 — produce a reproducible local build

The WASM artifact must be built with the same toolchain version that the
deployed binary used. The pinned toolchain in `rust-toolchain.toml` ensures
this, but only if you have not overridden it:

```bash
cargo build --release --target wasm32v1-none
```

The optimised artifact is written to
`target/wasm32v1-none/release/tricklepay_stream.wasm`.

**Why `wasm32v1-none`?** On Rust 1.82 and later, the familiar
`wasm32-unknown-unknown` target enables WASM `reference-types` and
`multi-value` features by default. The Soroban host rejects these
features, so builds targeting `wasm32-unknown-unknown` produce a WASM
module the network cannot execute. `wasm32v1-none` (Rust 1.84+) is the
supported target that avoids those extensions and produces a module the
Soroban environment accepts. If you build with the wrong target, Soroban
fails with a `WasmVm` error about unsupported `reference-types` or
`multi-value` features.

Compute its SHA-256 hash:

```bash
sha256sum target/wasm32v1-none/release/tricklepay_stream.wasm
```

To run one test while iterating on a focused change, pass the test name after
`cargo test`:

```bash
cargo test create_stream_locks_funds_and_assigns_id
```

The command is run from the workspace root and matches the test by name across
the workspace. To see output printed by a passing test, pass `--nocapture` to
the Rust test harness after `--`:

```bash
cargo test create_stream_locks_funds_and_assigns_id -- --nocapture
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

The script takes one required argument, the name of a Stellar CLI identity.
The network is optional. It defaults to `testnet` and is chosen with the
`NETWORK` environment variable, not a second argument. Run it from the
repository root, because the WASM path is relative:

```bash
# One-time setup: create an identity and fund it from friendbot.
stellar keys generate alice --network testnet --fund

# Deploy to testnet (the default).
./scripts/deploy.sh alice

# Deploy to another network configured in the Stellar CLI.
NETWORK=futurenet ./scripts/deploy.sh alice

# The same testnet deploy through make.
make deploy ID=alice
```

Running it without an identity prints the usage line and exits with status 1:

```text
usage: ./scripts/deploy.sh <identity-name>
```

On success the script prints its two progress lines, then the `cargo build`
output, then whatever the Stellar CLI logs while it uploads the WASM and
creates the contract. The last line on stdout is the new contract's address:

```text
Building optimized WASM...
   Compiling tricklepay-stream v... (...)
    Finished `release` profile [optimized] target(s) in ...
Deploying to testnet as 'alice'...
... (Stellar CLI transaction logs) ...
C...  (56-character contract address)
```

Save that `C...` address. It is the `<CONTRACT_ID>` you pass to every later
`stellar contract invoke` and to the verification steps in
[Verifying a deployment](#verifying-a-deployment). The script exits non-zero,
without deploying, if the build fails or the identity is unknown or unfunded.
### Step 2 — fetch the on-chain bytecode hash

Every contract uploaded to a Stellar network is stored as a Wasm entry keyed
by the SHA-256 hash of the bytecode. That hash is also recorded in the
contract's `instance` ledger entry as the `executable` field. Retrieve it with
the Stellar CLI:

```bash
# Replace <CONTRACT_ID> with the deployed bech32 contract address and
# <NETWORK> with "testnet", "mainnet", or a custom RPC URL.
stellar contract inspect --id <CONTRACT_ID> --network <NETWORK>
```

The output includes a `wasm_hash` field. This is the SHA-256 hash of the
bytecode the network is executing.

Alternatively, query the RPC directly:

```bash
stellar contract fetch --id <CONTRACT_ID> --network <NETWORK> \
  --out-file fetched.wasm
sha256sum fetched.wasm
```

### Step 3 — compare

If the hash from step 1 matches the `wasm_hash` from step 2, the live
contract was compiled from this exact source tree with the pinned toolchain.

**Caveat — reproducibility:** Rust WASM builds are not guaranteed to be
bit-for-bit reproducible across different host platforms, OS versions, or LLVM
releases, even when the same toolchain version is used. In practice, the pinned
toolchain in `rust-toolchain.toml` makes builds reproducible across Linux
hosts; macOS or Windows hosts may produce a hash that differs from the deployed
one even though the source is identical. If hashes do not match, try building
on a Linux host (or a Docker image with the pinned Rust toolchain) before
concluding that the deployment differs from the source.

## Project structure
A `cancel` call is rejected with `StreamAlreadyCompleted` if `now >= end_time`
— once the stream has fully vested there is nothing unvested to refund. A
stream that has already been cancelled cannot be cancelled again
(`AlreadyCancelled`).

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

### Amount ceiling

`create_stream` rejects any `total_amount` greater than **`i64::MAX`
(9 223 372 036 854 775 807 stroops, approximately 9.2 × 10¹⁸)**. This is not
an arbitrary policy limit — it is a safety bound derived from the vesting
arithmetic.

The vesting formula multiplies `total_amount` by an elapsed-time value before
dividing:

```
vested = total_amount * elapsed / duration
```

Both `total_amount` and `elapsed` are widened to `i128` for the multiplication.
`elapsed` can be at most `u64::MAX` seconds (the full range of the Soroban
ledger clock). For the product to stay within `i128::MAX` for every possible
elapsed value, `total_amount` must satisfy:

```
total_amount * u64::MAX ≤ i128::MAX
total_amount ≤ i128::MAX / u64::MAX ≈ 5.0 × 10¹⁸
```

`i64::MAX` (≈ 9.2 × 10¹⁸) is above that quotient, so the bound used in the
contract is slightly conservative, but it is the cleanest expressible limit and
is well above the total supply of any realistic token. A call that exceeds the
ceiling is rejected with `AmountTooLarge` before any tokens move.

**No-cliff example:** a stream of **1000 units over `[100, 1100]`** with
`cliff_time == start_time == 100` (no cliff):

| Time | `elapsed` | Exact share | Vested (truncated) |
| ---- | --------- | ----------- | ------------------ |
| 350  | 250       | 250.0       | 250                |
| 600  | 500       | 500.0       | 500                |
| 850  | 750       | 750.0       | 750                |
| 1100 | 1000      | 1000.0      | 1000               |

The schedule above divides evenly, so truncation has no visible effect. To see
it, consider **10 units over 

## Recent Changes
- Ongoing improvements and fixes as part of active development.
- See commit history and open issues for detailed change tracking.

## Recent Changes
- Ongoing improvements and fixes as part of active development.
- See commit history and open issues for detailed change tracking.