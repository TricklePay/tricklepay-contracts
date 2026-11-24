# Design: Topping Up an Existing Stream

**Status:** Accepted — implemented in [`topup_stream`](../contracts/stream/src/contract.rs#L344) with tests in [`topup.rs`](../contracts/stream/src/tests/topup.rs)  
**Related Issues:** #455, #635  
**Author:** Design Team  
**Date:** 2026-10-20 (Accepted 2026-11-20)

## Summary

This document defines the design for allowing the sender to add additional tokens to an existing stream, extending its duration or increasing its rate. This feature enables use cases such as extending a payroll stream, adding funds to a grant that is running low, or increasing a subscription payment without creating a new stream.

## Motivation

Currently, if a sender wants to increase the total value delivered through a stream, they must:

1. Cancel the existing stream (if not completed)
2. Create a new stream with the increased total
3. The recipient loses continuity (new stream id, new event history)

This workflow is disruptive and inefficient. A top-up mechanism would allow senders to extend or increase an existing stream without breaking its identity or requiring the recipient to track multiple stream ids.

## Design Challenges

Topping up a stream touches several core invariants and introduces semantic ambiguities that must be resolved before implementation:

### 1. Semantics of Top-Up

**What does "top up by X tokens" mean for the vesting schedule?**

Two primary interpretations exist:

#### Option A: Extend Duration (Preserve Rate)

Add tokens by extending `end_time`, keeping the vesting rate (tokens per second) constant.

**Example:** A stream of 1000 tokens over 1000 seconds (rate = 1 token/sec) is topped up by 500 tokens. The new `end_time` extends by 500 seconds, so the stream now delivers 1500 tokens over 1500 seconds at the same rate.

**Math:**
- Original: `total_amount = 1000`, `duration = end_time - start_time = 1000`, `rate = 1000 / 1000 = 1`
- After top-up: `total_amount = 1500`, `duration = 1500`, `rate = 1500 / 1500 = 1`
- `end_time' = end_time + (added_amount / rate)`

**Pros:**
- Intuitive for payroll/subscription use cases: "extend the stream by another month at the same rate"
- Preserves the vesting rate, which may be a contractual agreement

**Cons:**
- If the stream has already passed `end_time`, extending it into the future requires either moving `end_time` back in time (semantically weird) or starting a new vesting window from `now` (breaks continuity)
- Cliffs become meaningless after top-up (the cliff is already reached, so adding more tokens with the same cliff_time is a no-op)

#### Option B: Increase Rate (Preserve Duration)

Add tokens without changing `end_time`, increasing the vesting rate.

**Example:** A stream of 1000 tokens over 1000 seconds is topped up by 500 tokens. The `end_time` stays the same, so the stream now vests 1500 tokens over the original 1000-second window.

**Math:**
- Original: `total_amount = 1000`, `duration = 1000`, `rate = 1.0`
- After top-up: `total_amount = 1500`, `duration = 1000`, `rate = 1.5`

**Pros:**
- Simple state change: just increment `total_amount`
- Works for streams that have passed `end_time` (the added amount is immediately fully vested)
- No timeline manipulation required

**Cons:**
- Changes the vesting rate mid-stream, which may not match the sender's intent ("I want to extend, not accelerate")
- For a stream far from completion, the rate change may be imperceptible to the recipient

#### Recommendation: Option B — Increase Rate (Preserve Duration)

**The shipped design implements Option B in [`topup_stream`](../contracts/stream/src/contract.rs#L344): topping up increases `total_amount` while keeping `end_time` (and `start_time` / `cliff_time`) unchanged.**

**Why Option B shipped:**
- **Linear vesting formula stays intact:** Keeping `start_time`, `cliff_time`, and `end_time` fixed preserves `vested_amount = total_amount * elapsed / duration` without piecewise rate segments or cliff discontinuities.
- **Minimal state and interface surface:** Only `total_amount` changes, avoiding timeline manipulation and keeping a single unambiguous `topup_stream(id, added_amount)` entry point.
- **Clean lifecycle boundaries:** Paired with rejecting cancelled (`AlreadyCancelled`) and completed (`StreamAlreadyCompleted`, `now >= end_time`) streams, senders cannot retroactively alter finished streams.

**Option A (Extend Duration / Preserve Rate)** remains documented above and under [Alternatives Considered](#alternatives-considered) as the rejected alternative for `topup_stream`; supporting duration extension would require a separate entry point or explicit mode in the future.

### 2. Interaction with Vesting Formula

The vesting formula is:

```rust
vested_amount = total_amount * elapsed / duration
```

Where `elapsed = min(now, end_time) - max(start_time, cliff_time)`.

**Mid-Stream Top-Up Changes the Formula Retroactively:**

Consider a stream at `t = 500` (midpoint) of a 1000-second stream with `total_amount = 1000`. The vested amount is 500. If the sender tops up by 500 tokens (using Option B, preserving duration), the new `total_amount = 1500`, and the formula now says:

```rust
vested_amount = 1500 * 500 / 1000 = 750
```

The vested amount jumped from 500 to 750 instantly, without time passing. This is correct (the top-up increased the vested balance retroactively), but it means:

- The recipient can immediately withdraw the difference (250 in this example).
- Indexers computing `vested` from the original `Created` event will get the wrong answer until they see the `ToppedUp` event and recompute with the new `total_amount`.

**This is not a bug, but it is a semantic shift:** vested is no longer purely time-based; it also depends on the top-up history. Indexers must track `ToppedUp` events and adjust their vesting calculations accordingly.

### 3. Effect on `withdrawn` and `withdrawable`

The `withdrawn` field tracks how much the recipient has already claimed. After a top-up:

```rust
withdrawable = vested - withdrawn
```

If `vested` increases due to the top-up (as in the example above), `withdrawable` also increases, and the recipient can immediately withdraw the difference. This is expected behavior, but it must be tested to ensure no off-by-one errors or negative withdrawable balances occur.

**Edge Case: Top-Up After Full Withdrawal:**

If a stream is fully withdrawn at the midpoint (`withdrawn = vested = 500`), and then topped up by 500 tokens, the new `vested = 750` and `withdrawable = 750 - 500 = 250`. The recipient can withdraw again. This is correct.

**Edge Case: Top-Up at `end_time`:**

If a stream is at `end_time` with `vested = total_amount = 1000`, and then topped up by 500 tokens (using Option B), the new `total_amount = 1500` and `vested = 1500` (still 100% vested). The recipient can withdraw the additional 500 immediately. This is correct if the intent is "add bonus tokens that vest immediately." If the intent is "extend the stream," Option A (extend duration) would be more appropriate.

### 4. Interaction with Cancellation

**Can a cancelled stream be topped up?**

**Decision: No. Rejected with `StreamAlreadyCancelled` (or a new error variant if we want to be specific).**

**Reasoning:**

- A cancelled stream is frozen at the vested amount at cancellation time. The sender has already received their unvested refund.
- Topping up a cancelled stream would require un-freezing it, which means either:
  - (a) The sender can reclaim the top-up later via another cancel (but the first cancel already happened, so this creates a "partial un-cancel" state that is not well-defined).
  - (b) The top-up is non-refundable (the sender can never cancel again), which is inconsistent with the refund guarantee of active streams.
- Simpler rule: **cancelled streams cannot be topped up.** If the sender wants to add more tokens to a cancelled stream, they must create a new stream.

**Can an active stream be topped up and then cancelled?**

**Yes.** The top-up just increases `total_amount` (and possibly extends `end_time`, depending on the semantic). A subsequent `cancel` call works normally: the vested amount at cancellation time is frozen, and the sender gets back the unvested portion (including any unvested part of the top-up).

**Example:**
- Original stream: 1000 tokens over 1000 seconds.
- At `t = 500`, sender tops up by 500 tokens (using Option B: increase rate). New `total_amount = 1500`, `vested = 750`.
- At `t = 600`, sender cancels. `vested = 1500 * 600 / 1000 = 900`. Sender gets refund of `1500 - 900 = 600`.

The top-up is treated as part of the stream's total, and the cancellation refund is computed from the new `total_amount`. This is consistent with the existing cancellation logic.

### 5. Effect on Stream Status

- **Pending streams** (before `start_time`): Can be topped up. The top-up adds to `total_amount` (and possibly extends `end_time`), but the stream remains `Pending` until `start_time` is reached.
- **Streaming streams** (between `start_time` and `end_time`): Can be topped up. The vested amount may increase retroactively (see §2 above).
- **Completed streams** (`now >= end_time`, not cancelled): Can be topped up if using Option B (increase rate), in which case the added amount is immediately fully vested. If using Option A (extend duration), the `end_time` would move to the future, and the stream would become `Streaming` again. **This is semantically weird** (a completed stream becoming incomplete), so **completed streams should probably not be toppable**, or top-up should be rejected with a new error variant like `StreamAlreadyCompleted`.
- **Cancelled streams**: Cannot be topped up (see §4 above).

**Recommendation:** Reject top-up on completed or cancelled streams to avoid semantic ambiguity. The sender should create a new stream instead.

### 6. Interface Change

A new entry point is required:

```rust
/// Top up an existing stream by adding tokens.
///
/// Only the sender may call this. The added amount is transferred from the
/// sender into the contract, increasing the stream's total while keeping
/// `end_time` unchanged (Option B: increase rate, preserve duration).
///
/// Returns `Ok(())` on success.
///
/// # Authorization
///
/// - Requires authorization from the stream's `sender`.
///
/// # Rejections
///
/// - [`StreamError::StreamNotFound`] if `id` does not exist in storage.
/// - [`StreamError::InvalidAmount`] if `added_amount` is zero or negative.
/// - [`StreamError::AmountTooLarge`] if `stream.total_amount + added_amount > MAX_AMOUNT`.
/// - [`StreamError::AlreadyCancelled`] if the stream has been cancelled.
/// - [`StreamError::StreamAlreadyCompleted`] if the stream is completed (`now >= end_time`).
///
/// # Examples
///
/// ```ignore
/// client.topup_stream(&stream_id, &500);
/// ```
pub fn topup_stream(
    env: Env,
    id: u64,
    added_amount: i128,
) -> Result<(), StreamError>
```

**Validation Order:**

1. Authorization: `stream.sender` must authorize.
2. Stream existence: `StreamNotFound` if `id` does not exist.
3. Amount: `InvalidAmount` if `added_amount <= 0`, then `AmountTooLarge` if `stream.total_amount + added_amount > MAX_AMOUNT`.
4. State: `AlreadyCancelled` if `stream.cancelled`, then `StreamAlreadyCompleted` if `now >= stream.end_time`.

This order matches the precedent set by `create_stream` (authorization, then participants, then amount, then schedule, then capacity).

### 7. Event

A new event is published on every successful top-up:

```rust
/// Emitted when a stream is topped up.
///
/// Topics:
/// - `sender`: The stream's sender (indexed, for filtering by the party who added funds).
///
/// Data:
/// - `stream_id`: The stream that was topped up.
/// - `added_amount`: The amount of tokens added.
/// - `new_total_amount`: The stream's new `total_amount` after the top-up.
/// - `new_end_time`: The stream's new `end_time` after the top-up (if duration was extended; otherwise same as before).
///
#[contractevent]
pub struct ToppedUp {
    pub stream_id: u64,
    pub added_amount: i128,
    pub new_total_amount: i128,
    pub new_end_time: u64,
}
```

**Why include `new_end_time`?**
- If the semantic is "extend duration," `new_end_time` is critical for indexers to recompute the vesting schedule.
- If the semantic is "increase rate," `new_end_time` stays the same, but including it explicitly in the event avoids ambiguity.

**Ordering:** The event is published after the token transfer and state update, consistent with `Created` and `Withdrawn`.

### 8. Impact on Indexers

Indexers must handle `ToppedUp` events to correctly compute vested amounts:

1. **Recompute Vested:** After a `ToppedUp` event, the vested amount at any time `t` is computed using the new `total_amount` and new `end_time` (if changed).

2. **Track Top-Up History:** A stream can be topped up multiple times. Indexers should track the cumulative `total_amount` as the sum of the original `Created.total_amount` plus all `ToppedUp.added_amount` events, or simply use the latest `new_total_amount` from the most recent `ToppedUp` event.

3. **Withdrawable Balance:** After a top-up, `withdrawable = vested(new_total, new_duration, now) - withdrawn`. The `withdrawn` field is not reset by a top-up, so the indexer must subtract the cumulative withdrawn amount across all `Withdrawn` events, regardless of whether they occurred before or after the top-up.

4. **Gap Handling:** If an indexer encounters a `ToppedUp` event for a stream it has never seen a `Created` event for, it should call `get_stream(id)` to backfill the current state, which will reflect the post-top-up `total_amount` and `end_time`.

## Invariants

### Preserved Invariants

1. **Escrow Invariant:** The contract's token balance for a given token always equals the sum of `(total_amount - withdrawn)` across all streams using that token. Top-up increases `total_amount` by `added_amount` and transfers `added_amount` from the sender into the contract, so the escrow grows by exactly `added_amount`. The invariant holds.

2. **Monotonicity (with caveat):** For a fixed `total_amount`, vested is monotonic in time. After a top-up, `total_amount` changes, so vested may jump discontinuously at the instant of the top-up. But for any fixed `total_amount`, vested is still monotonic. This is not a violation; it is a state change that indexers must track.

3. **Authorization:** Only the sender can top up. Only the recipient can withdraw. Only the sender can cancel. These roles are unchanged.

### Changed Invariants

1. **`total_amount` Is No Longer Immutable After Creation:**
   - Before: `stream.total_amount` was set at `create_stream` and never changed (except by `cancel`, which freezes it at the vested amount).
   - After: `stream.total_amount` can increase via `topup_stream`. It can still be frozen by `cancel`, but the frozen value may now be higher than the original if a top-up occurred before cancellation.

2. **Vested Is No Longer Purely Time-Based:**
   - Before: `vested(t)` was a pure function of `t`, `start_time`, `end_time`, and the constant `total_amount`.
   - After: `vested(t)` also depends on the top-up history (the cumulative `total_amount` at time `t`). Indexers must track `ToppedUp` events to compute `vested` correctly.

### New Invariants

1. **Top-Up Increases `total_amount` and Optionally `end_time`:**
   - After a successful `topup_stream(id, added_amount)`, `stream.total_amount' = stream.total_amount + added_amount`.
   - If the semantic is "extend duration," `stream.end_time' = stream.end_time + (added_amount / rate)`.
   - If the semantic is "increase rate," `stream.end_time' = stream.end_time` (unchanged).

2. **Top-Up Does Not Alter `withdrawn`:**
   - `stream.withdrawn` is unaffected by `topup_stream`. The recipient's past withdrawals are still counted against the new, higher `total_amount`.

3. **Cancelled and Completed Streams Cannot Be Topped Up:**
   - `topup_stream` is rejected on a cancelled or completed stream. The sender must create a new stream instead.

## Tests Required

### Unit / Integration Tests

1. **Happy path:**
   - Create a stream, top it up, verify `total_amount` increased and (depending on semantic) `end_time` extended.
   - Top up a stream twice, verify cumulative `total_amount` is correct.

2. **Authorization:**
   - Only the sender can top up. Recipient calling `topup_stream` fails authorization.
   - Non-sender calling `topup_stream` fails authorization (before `StreamNotFound` check).

3. **Validation:**
   - Top up with zero or negative amount fails with `InvalidAmount`.
   - Top up that would exceed `MAX_AMOUNT` fails with `AmountTooLarge`.
   - Top up a nonexistent stream fails with `StreamNotFound`.
   - Top up a cancelled stream fails with `AlreadyCancelled`.
   - Top up a completed stream fails with `StreamAlreadyCompleted`.

4. **State consistency:**
   - After top-up, `get_stream(id).total_amount` reflects the new total.
   - After top-up, `vested(id)` may increase retroactively (depending on the semantic).
   - After top-up, `withdrawable(id)` may increase if the retroactive vesting exceeds past `withdrawn`.
   - The recipient can withdraw the increased `withdrawable` balance.

5. **Interaction with `cancel`:**
   - Top up a stream, then cancel. The refund is computed from the new `total_amount`.
   - Top up at midpoint, cancel immediately. The refund includes the unvested part of the top-up.

6. **Interaction with `withdraw`:**
   - Top up after a partial withdrawal. The recipient can withdraw the newly vested amount.
   - Top up after full withdrawal (at midpoint). The retroactive vesting makes `withdrawable > 0` again.

7. **Escrow accounting:**
   - After top-up, the contract's token balance increases by exactly `added_amount`.
   - After top-up and subsequent withdrawals/cancellations, the escrow invariant still holds.

8. **Event correctness:**
   - `ToppedUp` event is published with correct `stream_id`, `added_amount`, `new_total_amount`, and `new_end_time`.
   - Event ordering: the event is published after the state update.
   - No event is published if the call is rejected.

### Property-Based Tests

1. **Escrow invariant after top-up:**
   - For any stream, `contract.token.balance == sum(stream.total_amount - stream.withdrawn)` across all streams, before and after any top-up.

2. **Top-up is additive:**
   - `topup(id, A)` followed by `topup(id, B)` should have the same effect as `topup(id, A + B)` (assuming no other state changes in between).

3. **Vested never decreases:**
   - For any stream, `vested(t1) <= vested(t2)` for `t1 < t2`, even if a top-up occurs between `t1` and `t2`.

## Alternatives Considered

### Alt 1: No Top-Up Feature

**Rejected:** `topup_stream` was shipped (using Option B) so senders can add funds to an existing pending or active stream without cancelling and breaking stream continuity.

### Alt 2: Support Both "Extend Duration" and "Increase Rate"

**Rejected for initial release:** Two entry points (`topup_by_extending` and `topup_by_rate_increase`) or a parameter (`topup_stream(id, amount, mode)`) would allow the sender to choose, but doubles the test surface and indexer logic. Option B ("increase rate") shipped as the sole semantic in `topup_stream`, leaving Option A ("extend duration") documented as an out-of-scope alternative that could be added later as a separate entry point if demand arises.

### Alt 3: Allow Top-Up on Completed Streams

**Rejected:** This creates semantic ambiguity (does the stream become incomplete again?) and violates the intuition that a completed stream is "done." If the sender wants to add more tokens after completion, they should create a new stream.

### Alt 4: Reset `withdrawn` on Top-Up

**Rejected:** This would let the recipient withdraw the original vested balance a second time, which is incorrect. The `withdrawn` field tracks cumulative withdrawals across the stream's lifetime, and top-up does not change the past.

## Resolved Decisions

The open design questions were resolved as follows:

1. **Which semantic: extend duration or increase rate?**
   - **Decision: Increase rate (Option B).** Topping up increases `total_amount` while preserving the existing timeline bounds (`start_time`, `cliff_time`, `end_time`). This keeps the vesting formula `vested = total_amount * elapsed / duration` exact and linear without complex schedule distortion or discontinuity.

2. **Can pending streams be topped up?**
   - **Decision: Yes.** Pending streams (`now < start_time`) accept top-ups, increasing `total_amount` while the stream remains in `Pending` status until `start_time`.

3. **What error for top-up on a completed stream?**
   - **Decision: `StreamError::StreamAlreadyCompleted`.** Top-up on completed streams (`now >= end_time`) is rejected with `StreamError::StreamAlreadyCompleted` to prevent ambiguous retroactive vesting after completion.

4. **Should there be a minimum top-up amount?**
   - **Decision: No.** Any positive amount (`added_amount > 0`) is accepted. Non-positive amounts (`<= 0`) reject with `StreamError::InvalidAmount`.

5. **Can cancelled streams be topped up?**
   - **Decision: No.** Rejected with `StreamError::AlreadyCancelled`.

6. **Capacity bound:**
   - **Decision:** If `stream.total_amount + added_amount > MAX_AMOUNT`, rejected with `StreamError::AmountTooLarge`.

## Future Work

- **Top-Down (Reduction):** Allowing the sender to reduce `total_amount` (the opposite of top-up). This is more complex (what if `withdrawn > new_total_amount`?) and is out of scope.
- **Recipient-Initiated Top-Up:** Allowing the recipient to add their own tokens to a stream (e.g., co-funding a grant). Rejected for now due to authorization complexity.

## Changelog

- **2026-10-20:** Initial design proposal.
- **2026-11-20:** Accepted design with Option B (increase rate / preserve duration) and resolved open questions (#635).

## References

- [README.md § How a stream works](../README.md#how-a-stream-works)
- [contracts/stream/src/vesting.rs](../contracts/stream/src/vesting.rs) — vesting formula
- [docs/INTEGRATOR_OPERATIONS.md § Indexer Consistency Requirements](../docs/INTEGRATOR_OPERATIONS.md#indexer-consistency-requirements)
