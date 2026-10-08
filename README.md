# TricklePay Contracts

Soroban smart contracts for TricklePay, a token streaming protocol on Stellar.

A stream locks a sum of tokens from a sender and releases them to a recipient
linearly over time. The recipient can withdraw whatever has vested at any
moment; the sender can cancel and reclaim only the portion that has not yet
vested. This is the on-chain primitive behind payroll, vesting, grants, and
subscriptions, where value should move continuously rather than in lump sums.

**All stream data is public.** Every stream's participants, schedule, and
amounts are readable on-chain by anyone, not just the two parties involved —
see [THREAT_MODEL.md § What a third party can observe](THREAT_MODEL.md#what-a-third-party-can-observe)
before using this for payroll or any other arrangement where that information
is sensitive.

This repository holds the `stream` contract and its test suite. The indexer and
web client that build on it live in separate repositories; see
[Related repositories](#related-repositories).

Integrator-facing operational policies live in
[docs/INTEGRATOR_OPERATIONS.md](docs/INTEGRATOR_OPERATIONS.md). That guide
covers redeployment without upgradeability, interface stability, safe retry
behavior after uncertain submissions, and the practical duration limits imposed
by storage TTL.

A formal audit is the gate between this contract and production use (see
[SECURITY.md](SECURITY.md)). [docs/AUDIT_READINESS.md](docs/AUDIT_READINESS.md)
is the checklist of what must be in place — invariants, threat model, and test
coverage — before engaging an auditor.

Deliberate design trade-offs — things that look like missing features but
aren't — are recorded in [docs/DECISIONS.md](docs/DECISIONS.md), starting
with why there is no on-chain way to list a party's streams.

## Soroban SDK compatibility

The contract targets Soroban SDK `25.3.2`, pinned to an exact version (`=25.3.2`) in the workspace
`Cargo.toml`. The same version is used for the contract build and its test
host.

Upgrading the SDK can change generated contract clients and macros, the WASM
build, or host behavior exercised by tests. Review the SDK release notes,
update the workspace pin, and run the full test and lint suite. Also rebuild
and inspect the contract interface and WASM artifact before deployment, since
SDK changes can affect the contract ABI or what the Soroban host accepts.

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
  `[start_time, end_time]`; anything outside is rejected with `InvalidCliff`

  **A stream has no cliff when `cliff_time == start_time`.** There is no
  separate flag or null value to pass — the cliff is always a timestamp, and
  setting it to the start makes the gate vacuous. `vested_amount` withholds
  everything while `now < cliff_time || now < start_time`, so when the two are
  equal that reduces to `now < start_time`: exactly the start check every
  stream already applies. The no-cliff case is not special-cased anywhere in
  the vesting math, it simply falls out of the same expression. This is the usual default when a stream should begin vesting immediately from `start_time` rather than waiting for an explicit cliff. `cliff_time == end_time` is equally valid and withholds everything until the window closes — a pure lockup that vests in one step.

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

| Field          | Value                | Unix seconds |
| -------------- | -------------------- | ------------ |
| `total_amount` | 12 000 units         | —            |
| `start_time`   | 1 Jan 2025 00:00 UTC | `1735689600` |
| `cliff_time`   | 1 Apr 2025 00:00 UTC | `1743465600` |
| `end_time`     | 1 Jan 2026 00:00 UTC | `1767225600` |

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

_Before the cliff — 1 Feb 2025 (`now = 1738368000`):_

```
elapsed = 1738368000 - 1735689600 = 2678400 s  (31 days)
now < cliff_time  →  vested = 0
withdrawable = 0
```

One month has passed since the start and 1/12 of the total has accrued by the
linear schedule, but the cliff gate is still blocking it. Nothing can be
withdrawn yet.

_At the cliff — 1 Apr 2025 (`now = 1743465600`):_

```
elapsed = 1743465600 - 1735689600 = 7776000 s  (90 days)
vested  = 12000 * 7776000 / 31536000 = 2958 units  (truncated)
withdrawable = 2958
```

The cliff releases the entire 90 days of accrual in one step. The recipient
can withdraw up to 2958 units immediately, even though nothing was available
one second earlier.

_Six months in — 1 Jul 2025 (`now = 1751328000`):_

```
elapsed = 1751328000 - 1735689600 = 15638400 s  (181 days)
vested  = 12000 * 15638400 / 31536000 = 5950 units  (truncated)
withdrawable = 5950   // assuming nothing withdrawn yet
```

Vesting has continued linearly from the cliff. If the recipient withdrew the
2958 cliff lump on 1 Apr, the withdrawable balance at this point is
`5950 - 2958 = 2992` units.

**Summary table:**

| Date                                  | `now`        | Vested | Withdrawable (nothing taken yet) |
| ------------------------------------- | ------------ | ------ | -------------------------------- |
| 1 Feb 2025 (1 month in, before cliff) | `1738368000` | 0      | **0**                            |
| 1 Apr 2025 (cliff, 3 months in)       | `1743465600` | 2958   | **2958**                         |
| 1 Jul 2025 (6 months in)              | `1751328000` | 5950   | **5950**                         |
| 1 Jan 2026 (end)                      | `1767225600` | 12000  | **12000**                        |

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

| View               | Return value                                               | Reason                                             |
| ------------------ | ---------------------------------------------------------- | -------------------------------------------------- |
| `get_stream`       | stream record with `withdrawn = 1000`, `cancelled = false` | the record is never deleted                        |
| `withdrawable(id)` | `0`                                                        | `vested(1000) - withdrawn(1000) = 0`               |
| `vested(id)`       | `1000`                                                     | at or after `end_time`, the full amount has vested |
| `locked(id)`       | `0`                                                        | `total_amount(1000) - vested(1000) = 0`            |
| `progress(id)`     | `10000`                                                    | 100 % — fully vested                               |
| `status(id)`       | `Completed`                                                | `now >= end_time` and the stream was not cancelled |

Calling `withdraw` again after the balance is zero returns
`Err(NothingToWithdraw)` — nothing is transferred and nothing is recorded.

**How to tell a finished stream from a broken one.** A normally completed
stream has `status = Completed`, `locked = 0`, `progress = 10000`, and
`withdrawable = 0`. An incomplete or misconfigured stream would show a
non-zero `withdrawable` or `locked` alongside the same `Completed` status,
which means tokens remain unclaimed. The `get_stream` view exposes the raw
`withdrawn` and `total_amount` fields for a precise accounting check:
`withdrawn == total_amount` confirms the recipient has taken everything.

## Token interface

The `token` address passed to `create_stream` must implement the **SEP-41
Token Interface** — the same interface the Stellar Asset Contract (SAC)
implements for classic Stellar assets, and the one any custom Soroban token
should implement to be usable here. The contract talks to it through
`soroban_sdk::token::TokenClient` (`contracts/stream/src/contract.rs`).

**The only operation the contract calls is `transfer`.** It is invoked at four
points, always moving tokens to or from the contract's own address:

- `create_stream` — pulls `total_amount` from the sender into the contract.
- `withdraw` / `withdraw_amount` — pays the recipient their vested, unwithdrawn
  balance.
- `cancel` — refunds the sender whatever has not yet vested.

Nothing else on the token is called: no `balance`, `approve`, or `allowance`
check, and no admin or minting function. A token that implements `transfer`
correctly is sufficient for this contract regardless of what else it does or
doesn't support.

**Symptom of a non-conforming token.** The contract assumes `transfer` moves
exactly the requested amount, charges no undisclosed fee, and either succeeds
or fails atomically with no partial effect; it never re-checks balances
afterward. A token that violates this doesn't produce a typed `StreamError` —
there is no error variant for a bad token, because the failure is the token's,
not the stream contract's. Instead it shows up as behavior that looks like a
bug in this contract: a `withdraw` that reports success while the recipient
receives less than the vesting math promised (a token that short-transfers or
takes a fee), every call on a stream failing or reverting forever (a token
that always traps), or unexpected reentrant behavior around a transfer (a
token that calls back into this contract from within `transfer`). See
[THREAT_MODEL.md § Trust assumptions about the token contract](THREAT_MODEL.md#trust-assumptions-about-the-token-contract)
and [§ What happens when a token transfer fails](THREAT_MODEL.md#what-happens-when-a-token-transfer-fails)
for the full breakdown and who is responsible for choosing a conforming token.

## Development workflow

All common contributor tasks are wrapped in the `Makefile`. Run `make` (or
`make help`) from the repository root to list them:

```
  check      Run fmt-check, lint, and test — the same sequence CI runs.
             Use this before opening a pull request.
  build      Native debug build (cargo build).
  wasm       Optimised WASM artifact for deployment.
  test       Run the full test suite (cargo test).
  fmt        Format the workspace in place (cargo fmt).
  fmt-check  Verify formatting without modifying files (used in CI).
  lint       Lint every target and treat warnings as errors (cargo clippy -D warnings).
  audit      Audit dependencies for known vulnerabilities (cargo audit --deny warnings).
  clean      Remove build artifacts (cargo clean).
  deploy     Build, install, and deploy to testnet. Pass an identity: make deploy ID=alice
```

Quick reference for the most common tasks:

```bash
make check          # formatting + lints + tests (mirrors CI)
make test           # run the test suite only
make fmt            # auto-format all Rust source files
make wasm           # produce the release WASM ready for deployment
make audit          # check for vulnerable or unmaintained dependencies
make deploy ID=alice  # deploy to testnet using a Stellar CLI identity
```

> **Dependency audit:** `make audit` runs `cargo audit --deny warnings`.
> Some transitive Soroban test-host dependencies are allowlisted in
> `.cargo/audit.toml` because they are not compiled into the deployed WASM.
> See [`.cargo/AUDIT.md`](.cargo/AUDIT.md) for the full explanation.

### Reading the test suite

The suite is organised by behaviour, not by entry point, so a failing test
points at which part of the contract's rules broke rather than just which
function was called. It lives in two places:

- **`contracts/stream/src/vesting.rs`** has its own `#[cfg(test)] mod tests`
  testing the vesting arithmetic in isolation, with no contract or ledger
  environment involved: fixed-example unit tests
  (`nothing_vests_before_start`, `cliff_releases_accrued_amount_at_once`,
  `integer_division_rounds_down`) plus a `proptest!` block
  (`vested_between_zero_and_total`, `vested_is_monotonic_in_now`,
  `withdrawable_equals_vested_minus_withdrawn_when_withdrawn_le_vested`) that
  checks those same properties hold across randomly generated schedules and
  timestamps, not just the handful of examples above them.
- **`contracts/stream/src/tests/`** has the integration suite, run against
  the generated contract client, split into one file per functional area.
  The table below names where to look for each behaviour; it mirrors the
  module doc comment at the top of
  [`tests/mod.rs`](contracts/stream/src/tests/mod.rs), which is the
  authoritative, always-current version of this list.

| File              | Covers                                                                                           | Representative tests                                                                                                                      |
| ----------------- | ------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `creation.rs`     | `create_stream` validation: schedule and amount bounds, participant checks, id-counter exhaustion, deterministic validation order | `create_stream_rejects_invalid_parameters`, `create_stream_rejects_amount_above_max`, `create_stream_rejects_the_contract_as_recipient`, `create_stream_validation_order_is_deterministic`, `stream_id_counter_overflow_fails_closed` |
| `withdrawal.rs`   | `withdraw`/`withdraw_amount`: stepwise vesting, cliff gating, partial draws, exact-end and exact-cliff boundaries, double-withdraw and unknown-id guards | `withdraw_releases_vested_in_steps`, `cliff_blocks_withdrawal_until_reached`, `withdraw_amount_available_plus_one_receives_insufficient_balance`, `operations_on_unknown_stream_report_not_found` |
| `cancellation.rs` | `cancel`: the vested/refund split, boundary timing (start, cliff, end), rejecting a second cancel or one after completion | `cancel_refunds_unvested_and_preserves_vested`, `cancel_with_cliff_not_reached`, `cancel_on_stream_at_end_time_is_rejected`, `test_repeated_cancel_fails` |
| `views.rs`        | Read-only queries (`progress`, `locked`, `status`, etc.): monotonicity over time, correctness on a cancelled stream, no state or event side effects | `progress_never_decreases_as_time_advances`, `locked_never_goes_negative_across_sampled_times`, `views_are_correct_on_a_cancelled_stream`, `test_view_functions_publish_no_events` |
| `events.rs`       | Event ordering and topic correctness for every entry point, and silence on a rejected call        | `create_emits_created_after_the_funding_transfer`, `lifecycle_events_follow_operation_order`, `created_event_topics_index_sender_and_recipient`, `rejected_create_publishes_no_events` |
| `storage.rs`      | `DataKey` encoding across the id range, and persistent/instance TTL bumps on both sides of `BUMP_THRESHOLD` | `stream_ids_map_to_distinct_persistent_keys`, `a_stream_entry_ttl_decays_with_the_ledger_sequence`, `withdrawing_restores_a_decayed_stream_ttl`, `view_calls_do_not_extend_the_instance_ttl` |
| `auth.rs`         | Authorization requirements for every mutating entry point                                         | `withdraw_requires_recipient_authorization`, `cancel_requires_sender_authorization`, `test_third_party_cannot_mutate_streams`              |
| `helpers.rs`      | Not tests itself — the `StreamTest` fixture and clock helpers (`set_time`, `set_sequence`) every file above builds on | —                                                                                                                                             |

If you are trying to understand a specific rule — what exactly happens at an
exact cliff, how a cancellation splits funds, what a rejected call leaves
behind — the fastest path is usually to search these files for the behaviour
by name rather than re-reading `contract.rs`; the test names are written to
describe the rule, not the function under test. See
[CONTRIBUTING.md § Writing a test for a new behaviour](CONTRIBUTING.md#writing-a-test-for-a-new-behaviour)
for the conventions a new test should follow.

### Measuring the compiled contract size

The compiled WASM is what actually gets installed on-chain, and Soroban's
install and per-invocation resource fees scale with it — a larger contract
costs more to deploy and more to call than a smaller one. This is why the
`[profile.release]` section of `Cargo.toml` is tuned specifically for size
(`opt-level = "z"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`,
`strip = "symbols"`, `debug = 0`) rather than for compile speed or runtime
performance; see the build-tooling entry in [CHANGELOG.md](CHANGELOG.md) for
the full rationale behind each setting.

Measure it after any change that touches the contract's code, especially
before a deployment:

```bash
make wasm
ls -la target/wasm32v1-none/release/tricklepay_stream.wasm

# or, for just the byte count:
wc -c < target/wasm32v1-none/release/tricklepay_stream.wasm
```

**Current size:** as of 2026-10-03, the release WASM built from this source
is **40,808 bytes** (≈ 40 KB). There is no enforced ceiling checked in CI, so
this number is a reference point, not a budget — treat a jump that isn't
explained by an intentional feature addition as a regression worth
investigating before merging, the same way `make audit` and `make check`
surface other classes of regressions.

## Implementation details

### Ledger time as the source of truth

Vesting uses the **ledger close time** (`env.ledger().timestamp()`) rather than wall clock time. This means vesting advances in discrete steps tied to ledger closes, not continuously. The Stellar ledger closes roughly every 5 seconds, so a stream's vested balance jumps forward in 5-second increments rather than updating smoothly every millisecond.

**Practical implications:**

- A client showing a countdown or live vesting progress bar should be aware that the on-chain state updates in ledger-sized steps. A smooth countdown on the client side is a UI convenience — it does not reflect how the contract sees time.
- **No off-chain process or keeper is required.** The contract reads the current ledger time whenever a view or mutating call is made, so vesting progresses automatically without any external automation or scheduled job.
- When you call `vested(id)` or `withdrawable(id)`, the result is computed from the ledger timestamp at that instant. Two calls in the same transaction see the same time; two calls in different ledgers may see different vested amounts even if only seconds have elapsed.

**Reference point for clients:** When deriving remaining time or vested amount in your application, use the ledger timestamp returned by the network as your reference point, not the local system clock. Query the ledger time alongside the stream state to ensure your calculations match what the contract will report on the next call.

### Deriving the remaining time

To show how much time is left in a stream, a client must compute the remaining duration from the stream's schedule fields. Doing this consistently with the contract's own arithmetic requires using the same reference point: the **ledger close time** (`env.ledger().timestamp()`), not the client's wall clock.

**Formula:**

```
remaining_seconds = end_time - now
```

where `now` is the current ledger timestamp. If `now >= end_time`, the stream is fully vested and `remaining_seconds` is zero (or negative, which should be clamped to zero for display).

**Worked example:**

Using a stream with `start_time = 1735689600` (1 Jan 2025), `end_time = 1738368000` (1 Feb 2025), queried at ledger time `now = 1737072000` (17 Jan 2025):

```
remaining_seconds = 1738368000 - 1737072000 = 1296000 seconds
remaining_days = 1296000 / 86400 = 15 days
```

The stream has 15 days remaining until it is fully vested.

**Important:** Always query the ledger time from the Stellar network (via an RPC call or by inspecting the latest ledger header) and use that value as `now`. Using `Date.now()` or a local system clock introduces skew — your countdown might say "5 seconds left" while the contract still reports 10 seconds, or vice versa, because your clock differs from the validator's consensus time.

**Converting to human-readable units:**

```
remaining_days = remaining_seconds / 86400
remaining_hours = (remaining_seconds % 86400) / 3600
remaining_minutes = (remaining_seconds % 3600) / 60
```

All division is integer division; the remainder is discarded. For a smooth countdown in a UI, you can interpolate between ledger updates, but always resync to the true ledger time on each new ledger close to avoid drift.

### Rounding direction

Integer division in the vesting formula **truncates** (rounds down), which means fractional tokens are never credited early. The formula `total_amount * elapsed / duration` discards any remainder from the division, so the vested amount is always the floor of the mathematically exact value.

**Which party does this favor?**

Rounding down **favors the sender** (and delays the recipient). A fractional token that has partially vested is not yet withdrawable by the recipient. It only becomes available once enough additional time has passed to push the vested amount over the next whole-token boundary.

**Maximum size of the difference:**

The rounding error per calculation is **less than 1 unit** of the token. Because the contract never adds a fractional result to `withdrawn` or anywhere else, the difference between the true mathematical vested amount and the truncated integer amount is bounded by `0 ≤ error < 1`. Over the life of a stream, this sub-unit remainder may accumulate across multiple vesting steps, but the recipient receives the correct total by the `end_time` because the final calculation `vested_amount(total_amount, start_time, end_time, cliff_time, end_time)` returns exactly `total_amount` with no division.

**Practical impact:**

For a token with 7 decimals (like USDC on Stellar, where 1 USDC = 10⁷ stroops), the maximum rounding difference is 0.0000001 USDC — economically negligible. For a stream of 1,000 tokens over 1,000,000 seconds, the recipient might be short by a fraction of a stroop at any given instant, but will receive the full 1,000 tokens by the end.

**Example:**

Stream of 10 tokens over 3 seconds, queried at `now = start_time + 1`:

```
vested = 10 * 1 / 3 = 3.333... → truncated to 3
```

The recipient can withdraw 3 tokens, not 3.333. The remaining 0.333 tokens are still locked and will vest as time progresses. At `now = start_time + 2`:

```
vested = 10 * 2 / 3 = 6.666... → truncated to 6
```

At `end_time`:

```
vested = 10 * 3 / 3 = 10 (exact, no truncation)
```

The truncation never causes the recipient to lose tokens — it only delays when fractional amounts become available.

### Full amount escrowed at creation

When a stream is created, the **entire `total_amount` is transferred from the sender to the contract** in the same transaction. This is a deliberate design choice that removes the need for the recipient to trust the sender's future behavior.

**Why escrow the full amount up front?**

1. **Trustless guarantee for the recipient:** The recipient is guaranteed that the tokens exist and are locked in the contract for the duration of the stream. There is no risk that the sender will fail to pay, run out of funds, or renege on the agreement. The vested amount is always claimable, regardless of what the sender does afterward.

2. **Simplifies the contract:** Because the contract holds the full amount, it does not need to handle partial funding, top-ups, or the complexity of a sender failing to deliver. Every stream is fully funded from the moment it is created, so the vesting logic is purely time-based arithmetic with no external dependencies.

3. **Enables cancellation refunds:** The sender can cancel a stream and immediately receive the unvested remainder back. This is only possible because the full amount is already in the contract — there is no need to track or enforce future payments.

**Trade-off: capital cost to the sender:**

The sender must **lock the full amount up front**, which means that capital is not available for other uses during the stream's lifetime. For a one-year stream of 120,000 tokens, the sender must have 120,000 tokens free at creation time, even though the recipient will draw them down gradually over the year.

This is the price of the trustless guarantee. The sender is giving up liquidity in exchange for the ability to cancel and reclaim the unvested portion at any point. If the sender cannot afford to lock the full amount, a stream is not the right primitive — a manual payment schedule or a different escrow arrangement would be needed instead.

**Comparison to alternatives:**

- **Pay-as-you-go:** The sender transfers tokens at regular intervals (e.g., monthly payroll). This preserves sender liquidity but requires the recipient to trust that future payments will arrive. If the sender stops paying, the recipient has no recourse.
- **Incremental escrow:** The contract pulls tokens from the sender as they vest. This reduces the sender's locked capital but adds complexity: the contract must handle insufficient balances, failed transfers, and partial funding. It also weakens the recipient's guarantee — tokens might not be available when they vest.
- **Full escrow (this design):** The sender locks everything up front, the recipient is guaranteed every vested token, and the contract logic is simple and trust-minimized. The sender retains the right to cancel and reclaim unvested funds, so the capital is not fully at risk.

The full-escrow model is the right fit for use cases where the recipient needs a strong guarantee (employee compensation, vesting grants, subscription prepayment) and the sender can afford to lock the capital. For scenarios where liquidity is more important than trustlessness, a different mechanism would be more appropriate.

### `locked`: a precise figure, not a synonym for "still escrowed"

"Locked" and "unvested" get used interchangeably in conversation, but
`locked(id)` reports one specific number:

```
locked = max(total_amount - vested_amount(now), 0)
```

This is **the unvested remainder of the schedule** — the portion time has
not reached yet — not "everything this stream still holds in the contract."
Those are two different figures that happen to be equal only part of the
time.

**How it relates to the escrowed balance.** At any moment, what a stream
still has sitting in the contract is `total_amount - withdrawn`, and that
splits into two independent pieces:

- `locked` — vested amount, not yet claimable by anyone.
- `withdrawable` — vested but not yet withdrawn, claimable by the recipient
  right now.

`locked + withdrawable == total_amount - withdrawn` holds at every point in
an active stream's life (this is exactly the split `vesting::settlement`
computes for `cancel`). A fully vested stream whose recipient has not yet
withdrawn reports `locked = 0` while still holding its entire remaining
balance in escrow — that balance shows up in `withdrawable`, not `locked`.
So `locked == 0` means "nothing left to vest," never "nothing left in the
contract."

`locked` is also exactly what `cancel` would refund the sender right now —
cancellation returns the unvested remainder, which is this figure at the
instant of cancellation.

**How cancellation changes what `locked` means.** `cancel` refunds the
current `locked` value to the sender, then freezes the stream: `total_amount`
is cut down to the vested amount at that instant and `end_time` is set to
`now` (see
[THREAT_MODEL.md § Invariants](THREAT_MODEL.md#invariants)). Recomputing
`vested_amount` for any later time then returns that same frozen total, so
`locked` is `0` forever after — but for a different reason than it was `0`
at full vesting: there is no schedule left to run, not because the contract
already paid out everything for that stream. A cancelled stream's
vested-but-unwithdrawn balance, if the recipient has not claimed it yet,
still exists in escrow and is reported by `withdrawable`, not `locked`.

## Events

Every mutating entry point publishes a Soroban event when it succeeds. Rejected
calls publish nothing. Indexers and off-chain consumers use these events to
track stream state without follow-up `get_stream` calls, and to filter streams
by participant address.

**Field order is part of the interface.** The Soroban event encoding preserves
declaration order, so reordering fields in any event struct is a breaking change
for downstream consumers — treat it with the same care as renaming a field or
changing its type.

Each event has a set of _topics_ (marked `#[topic]`) that the network indexes
for efficient filtering, and a _data_ payload containing the remaining fields.
Topics appear first in the encoding; the data fields follow in declaration order.

### `Created`

Emitted by `create_stream` when a new stream is opened successfully.

| Field          | Kind  | Type      | Description                                                                                                |
| -------------- | ----- | --------- | ---------------------------------------------------------------------------------------------------------- |
| `sender`       | topic | `Address` | The address that funded the stream                                                                         |
| `recipient`    | topic | `Address` | The address that will receive the vested tokens                                                            |
| `id`           | data  | `u64`     | The id assigned to the new stream                                                                          |
| `token`        | data  | `Address` | The token contract address                                                                                 |
| `total_amount` | data  | `i128`    | Total tokens locked into the stream                                                                        |
| `start_time`   | data  | `u64`     | Unix timestamp (seconds) when vesting begins                                                               |
| `end_time`     | data  | `u64`     | Unix timestamp (seconds) when the stream is fully vested                                                   |
| `cliff_time`   | data  | `u64`     | Unix timestamp (seconds) before which nothing can be withdrawn; equals `start_time` when there is no cliff |

Indexers can subscribe on the `sender` or `recipient` topics to receive all
streams for a given address without scanning every event.

### `Withdrawn`

Emitted by both `withdraw` and `withdraw_amount` when tokens are transferred to
the recipient.

| Field       | Kind  | Type      | Description                               |
| ----------- | ----- | --------- | ----------------------------------------- |
| `recipient` | topic | `Address` | The address that received the tokens      |
| `id`        | data  | `u64`     | The stream that was drawn from            |
| `amount`    | data  | `i128`    | Number of tokens transferred in this call |

### `Cancelled`

Emitted by `cancel` when a sender stops a stream early. Both sides of the split
are included so an indexer can record the final state without a follow-up
`get_stream` call.

| Field              | Kind  | Type      | Description                                                       |
| ------------------ | ----- | --------- | ----------------------------------------------------------------- |
| `sender`           | topic | `Address` | The address that cancelled the stream and received the refund     |
| `id`               | data  | `u64`     | The stream that was cancelled                                     |
| `recipient_amount` | data  | `i128`    | Vested tokens still claimable by the recipient after cancellation |
| `sender_refund`    | data  | `i128`    | Unvested tokens immediately refunded to the sender                |

Note that `recipient_amount` reflects the _remaining claimable balance_ at
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

The suite covers the vesting math in isolation and the contract end to end,
including the storage and event behaviour described above. See
[Reading the test suite](#reading-the-test-suite) for which file and which
named test covers a given behaviour.

## Deploying to testnet

`scripts/deploy.sh` wraps the Stellar CLI to build, install, and deploy the
contract. It expects a funded identity configured with `stellar keys`.

### What the deploying identity needs, and what it controls afterward

**What it requires.** The identity used to deploy must be a Stellar account
that already exists and holds enough native balance to cover the
transaction's base fee and the one-time resource fee for installing the WASM
and creating the contract instance. `stellar keys generate ... --fund` (shown
below) satisfies this on testnet by creating the account and funding it from
friendbot in one step; on mainnet the account must be funded through an
ordinary payment before it can deploy anything. Nothing else is required —
the identity does not need any pre-existing relationship with this contract,
and does not need to hold the token that will later be streamed.

**What the key controls afterward: nothing contract-specific.** This
contract has no admin, owner, or upgrade entry point (see
[THREAT_MODEL.md § No pause mechanism](THREAT_MODEL.md#no-pause-mechanism)
and [§ Immutability](THREAT_MODEL.md#immutability)), so deploying it does not
make the deploying identity a privileged account. `create_stream`,
`withdraw`, and `cancel` all authorize against the `sender`/`recipient`
addresses stored on each individual stream (see
[THREAT_MODEL.md § Authorization model](THREAT_MODEL.md#authorization-model)),
never against whoever submitted the deployment transaction. Once the deploy
transaction lands, the deploying key has exactly the same authority over the
contract as any other Stellar account — none — unless that same identity is
later also named as a `sender` or `recipient` on a specific stream, in which
case it has the authority that role carries, like any other address would.

**Protect the key anyway.** Even though it holds no contract privilege
afterward, the deploying identity is still a real, funded Stellar account,
and the same account is often reused to deploy again later. Treat its
custody the same way you would any other signing key that controls a stream
participant — see
[THREAT_MODEL.md § Key compromise](THREAT_MODEL.md#out-of-scope-risks) for
what a compromised key can do, and the
[Stellar CLI identity documentation](https://developers.stellar.org/docs/tools/cli)
for how `stellar keys` stores and manages keys locally.

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
[Verifying a deployment](#verifying-a-deployment). Record the deployment details using the [deployment record template](docs/DEPLOYMENT_RECORD_TEMPLATE.md). The script exits non-zero,
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

### Deliberately omitted validations

The contract deliberately does not validate some inputs that might look questionable, to ensure it does not break legitimate use cases:

- **Sender and recipient being the same address:** A stream from an address to itself has the effect of locking the sender's own tokens and handing them back over time. This is a legitimate way to self-stream funds.
- **Token acting as a participant:** A token contract acting as the `sender` or `recipient` is permitted. Rejecting this could break legitimate smart contract composability.
- **Start time in the past:** Accepted as long as `end_time` is in the future. The elapsed portion vests immediately, which is useful for backdating streams.

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
it, consider 10 units over 3 seconds, where 10 * 1 / 3 = 3.333… truncates to 3.

## Recent Changes

- Ongoing improvements and fixes as part of active development.
- See commit history and open issues for detailed change tracking.

## Recent Changes

- Ongoing improvements and fixes as part of active development.
- See commit history and open issues for detailed change tracking.
