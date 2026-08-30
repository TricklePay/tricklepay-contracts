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
it, consider **10 units over 
