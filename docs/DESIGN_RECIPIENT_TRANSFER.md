# Design: Transferring the Right to Receive a Stream

**Status:** Accepted  
**Related Issues:** #456  
**Author:** Design Team  
**Date:** 2026-10-20

## Summary

This document defines the design for allowing the recipient of a stream to transfer their receiving rights to a new address. This feature enables use cases such as selling a future income stream, delegating payment collection, or reassigning a vesting schedule without requiring the sender's involvement.

## Motivation

Currently, the recipient address is immutable once a stream is created. If a recipient wants to delegate their stream to another party (e.g., selling their vesting rights, assigning payroll to a different account, or moving to a new address), they must:

1. Withdraw all currently vested funds
2. Ask the sender to cancel the stream
3. Have the sender create a new stream to the new recipient

This workflow requires sender cooperation and breaks stream continuity. A transfer mechanism would allow recipients to reassign their rights unilaterally, similar to how financial instruments can be assigned or sold.

## Design

### Entry Point

Add a new mutating entry point:

```rust
/// Transfer recipient rights to a new address.
///
/// Only the current recipient may call this. The new recipient gains all
/// rights to withdraw vested amounts. The stream's `withdrawn` balance and
/// vesting schedule remain unchanged. The sender is not notified and their
/// cancellation rights are unaffected.
///
/// Returns `Ok(())` on success.
///
/// # Authorization
///
/// - Requires authorization from the stream's current `recipient`.
///
/// # Rejections
///
/// - [`StreamError::StreamNotFound`] if `id` does not exist in storage.
/// - [`StreamError::InvalidParticipant`] if `new_recipient` is the stream
///   contract's own address, or if `new_recipient == stream.recipient`
///   (transferring to oneself is a no-op and is rejected to avoid confusion).
///
/// # Examples
///
/// ```ignore
/// client.transfer_recipient(&stream_id, &new_address);
/// ```
pub fn transfer_recipient(
    env: Env,
    id: u64,
    new_recipient: Address,
) -> Result<(), StreamError>
```

### Implementation Notes

1. **Authorization:** Only the current `recipient` may invoke this. The sender has no authority over recipient transfers.

2. **Validation:**
   - The `new_recipient` cannot be the stream contract's own address (same rule as `create_stream`).
   - Transferring to the same address (`new_recipient == stream.recipient`) is rejected with `InvalidParticipant` to prevent no-op transactions.
   - No check is performed on whether `new_recipient == stream.sender`. A recipient transferring rights back to the sender is permitted (enables refund-like scenarios).

3. **State Changes:**
   - Update `stream.recipient` to `new_recipient`.
   - All other fields (`sender`, `token`, `total_amount`, `withdrawn`, `start_time`, `end_time`, `cliff_time`, `cancelled`) remain unchanged.
   - The vesting schedule continues uninterrupted.

4. **Effect on Other Operations:**
   - **`withdraw` / `withdraw_amount`:** The new recipient can immediately withdraw any vested balance. The old recipient loses all withdrawal rights.
   - **`cancel`:** The sender's cancellation rights are unaffected. The sender can still cancel the stream, and the refund still goes to the sender. The `Cancelled` event will reference the new recipient as the party whose remaining claimable balance is frozen.
   - **Views:** All view functions (`get_stream`, `withdrawable`, `vested`, etc.) immediately reflect the new recipient.

5. **No Token Movement:** `transfer_recipient` does not move any tokens. Vested but unwithdrawn amounts remain in the contract until the new recipient withdraws them.

### Event

A new event is published on every successful transfer:

```rust
/// Emitted when a stream's recipient is transferred.
///
/// Topics:
/// - `old_recipient`: The previous recipient (indexed, for filtering by the party who gave up rights).
/// - `new_recipient`: The new recipient (indexed, for filtering by the party who gained rights).
///
/// Data:
/// - `stream_id`: The stream whose recipient was changed.
///
#[contractevent]
pub struct RecipientTransferred {
    pub stream_id: u64,
}
```

**Topic Indexing:**
- `old_recipient` and `new_recipient` are both indexed topics so indexers can filter streams by "recipient gave away" or "recipient received" without scanning every event.
- This mirrors the pattern used in `Created` (which indexes `sender` and `recipient`) and `Withdrawn` / `Cancelled` (which index `recipient` / `sender` respectively).

**Ordering:** The event is published after the state update (`set_stream`) but before the function returns, consistent with the existing event ordering for `withdraw` and `cancel`.

### Impact on Indexers

Indexers tracking stream ownership must handle `RecipientTransferred` events:

1. **Recipient History:** An indexer that answers "which streams does address X have?" must now update its recipient mapping on every `RecipientTransferred` event, not just on `Created`.

2. **Ownership Chain:** A stream can change recipients multiple times. Indexers should track the current recipient as the one in the most recent `RecipientTransferred` event for that `stream_id`, or fall back to the `Created` event's `recipient` if no transfer has occurred.

3. **Gap Handling:** If an indexer encounters a `RecipientTransferred` event for a stream id it has never seen a `Created` event for, it should call `get_stream(id)` to backfill the current recipient and schedule, rather than assuming the indexer's state is complete. This is the same pattern documented in [docs/INTEGRATOR_OPERATIONS.md § Detect and backfill gaps](../docs/INTEGRATOR_OPERATIONS.md#detect-and-backfill-gaps).

4. **Event Filtering:** The indexed `old_recipient` and `new_recipient` topics allow efficient filtering. An indexer can subscribe to:
   - "All streams transferred away from address A" (filter on `old_recipient == A`)
   - "All streams transferred to address B" (filter on `new_recipient == B`)
   - "All streams where address C was ever involved" (union of `sender`, `recipient`, `old_recipient`, `new_recipient` across all events)

### Invariants

Existing invariants are preserved:

1. **Escrow invariant:** The contract's token balance for a given token always equals the sum of `(total_amount - withdrawn)` across all streams using that token. `transfer_recipient` does not move tokens or alter `total_amount` or `withdrawn`, so the escrow remains unchanged.

2. **Monotonicity:** Vesting is still monotonic in ledger time for a given stream. Changing the recipient does not affect `start_time`, `end_time`, `cliff_time`, or the vesting formula.

3. **Cancellation:** The sender can still cancel. The unvested refund still goes to the sender. The frozen vested balance is claimable by whoever is the recipient at cancellation time (the new recipient if a transfer occurred).

4. **Authorization:** Only the current `recipient` (after any transfers) can withdraw. Only the original `sender` can cancel. The sender cannot transfer the sending rights, and the recipient cannot transfer the cancellation rights.

### New Invariants

1. **Recipient uniqueness at any instant:** At any given ledger time, a stream has exactly one recipient. `transfer_recipient` atomically replaces the old recipient with the new one; there is no intermediate state where both or neither are the recipient.

2. **Transfer is immediate and irreversible:** Once `transfer_recipient` succeeds, the old recipient has zero rights. There is no cooldown, no approval from the new recipient, and no way for the old recipient to reclaim the stream unless the new recipient transfers it back.

## Tests Required

### Unit / Integration Tests

1. **Happy path:**
   - Create a stream, transfer the recipient, verify the new recipient can withdraw and the old recipient cannot.
   - Create a stream, transfer the recipient twice, verify the final recipient is correct.

2. **Authorization:**
   - Attempting to transfer from an address other than the current recipient fails authorization (before `StreamNotFound` check).
   - The sender cannot call `transfer_recipient` on their own stream.

3. **Validation:**
   - Transferring to the contract's own address fails with `InvalidParticipant`.
   - Transferring to the same recipient (`new_recipient == stream.recipient`) fails with `InvalidParticipant`.
   - Transferring to the sender is allowed (no error).
   - Transferring a nonexistent stream id fails with `StreamNotFound`.

4. **State consistency:**
   - After transfer, `get_stream(id).recipient` returns the new recipient.
   - After transfer, `withdrawable(id)` still returns the same vested amount (no tokens moved).
   - The old recipient's `withdraw(id)` call fails authorization.
   - The new recipient's `withdraw(id)` call succeeds.

5. **Interaction with `cancel`:**
   - Transfer the recipient, then have the sender cancel. The `Cancelled` event should reference the new recipient, and the vested balance should be claimable by the new recipient.
   - Transfer the recipient after a partial withdrawal. The new recipient can withdraw the remaining vested balance but not the amount already withdrawn by the old recipient.

6. **Interaction with a cancelled stream:**
   - Transfer the recipient on a stream that is already cancelled. The new recipient can still withdraw any vested-but-unwithdrawn balance.
   - A fully withdrawn and cancelled stream can still be transferred (though there is nothing left to withdraw).

7. **Multiple transfers:**
   - Transfer A → B, then B → C. Verify A cannot withdraw, B cannot withdraw, and C can withdraw.
   - Transfer A → B → A (round trip). Verify A regains withdrawal rights.

8. **Event correctness:**
   - `RecipientTransferred` event is published with correct `stream_id`, `old_recipient`, and `new_recipient` topics.
   - Event ordering: the event is published after the state update (a `get_stream` call in the same transaction after the transfer sees the new recipient).
   - No event is published if the call is rejected (e.g., `InvalidParticipant` or `StreamNotFound`).

### Property-Based Tests

1. **Transfer is idempotent to the stream's vesting state:**
   - For any stream and any ledger time `t`, `vested(id, t)` and `withdrawable(id, t)` are the same before and after a recipient transfer.

2. **Withdrawal authorization follows the current recipient:**
   - After any sequence of transfers, only the most recent `new_recipient` can successfully call `withdraw`.

3. **The sender's cancellation rights are unaffected:**
   - For any stream and any sequence of recipient transfers, the original `stream.sender` can still cancel (unless the stream is already cancelled or completed).

## Security Considerations

1. **No Recipient Approval:** The new recipient is not asked to accept the transfer. This mirrors the `create_stream` design, where the recipient is not asked to approve a stream being opened to them. A malicious actor cannot harm the new recipient by transferring a stream to them (the worst case is the new recipient ignores it), but the new recipient should be aware they now control a claimable balance.

2. **Old Recipient Has No Recourse:** Once transferred, the old recipient cannot reclaim the stream. This is intentional (the transfer is a unilateral assignment), but integrators should warn users that the operation is irreversible. A UI should require explicit confirmation before invoking `transfer_recipient`.

3. **Sender Is Not Notified:** The sender's cancellation rights and refund destination are unchanged, but they have no on-chain visibility into recipient transfers unless they query the stream or monitor events. A sender who wants to know who they are currently streaming to must call `get_stream(id).recipient` or subscribe to `RecipientTransferred` events.

4. **No Tokens Move During Transfer:** The vested balance stays in the contract. The new recipient must call `withdraw` to claim it. If the new recipient address is inaccessible (lost key, contract with no withdraw mechanism), the tokens become unclaimable. This is the same risk as creating a stream to an inaccessible recipient in the first place.

## Alternatives Considered

### Alt 1: Require Sender Approval

**Rejected:** This would defeat the purpose of unilateral assignment. The recipient should be able to reassign their rights without involving the sender, just as they can sell or assign a financial instrument without the issuer's permission.

### Alt 2: Require New Recipient Approval

**Rejected:** This adds complexity (two-phase transfer with an acceptance step) and is inconsistent with `create_stream`, which also does not require recipient approval. A recipient who does not want the stream can simply ignore it or transfer it elsewhere.

### Alt 3: Allow Transferring to the Same Address (No-Op)

**Rejected:** Allowing `new_recipient == stream.recipient` would succeed but do nothing, wasting gas and potentially confusing users. Rejecting it with `InvalidParticipant` makes it clear the call had no effect and prevents accidental no-op transactions.

### Alt 4: Emit the Event Before the State Update

**Rejected:** This would break the ordering invariant established by `withdraw`, `cancel`, and `create_stream`, which all publish events after state changes. Indexers expect events to reflect committed state.

## Future Work

- **Sender Transfer:** Allowing the sender to transfer their cancellation and refund rights to a new address. This is a separate feature with different authorization and event requirements.
- **Transfer Hooks:** An optional callback to the new recipient to notify them of the transfer. Rejected for now to keep the contract simple and avoid reentrancy risks.
- **Transfer Fees:** Charging a fee (to the contract, to the sender, or to a DAO) on each transfer. Rejected for now as it introduces economic complexity and admin-like parameters.

## Changelog

- **2026-10-20:** Initial design proposal.

## References

- [README.md § How a stream works](../README.md#how-a-stream-works)
- [docs/INTEGRATOR_OPERATIONS.md § Indexer Consistency Requirements](../docs/INTEGRATOR_OPERATIONS.md#indexer-consistency-requirements)
- [docs/DECISIONS.md § ADR 1: Enumeration is off-chain, not an entry point](../docs/DECISIONS.md#adr-1-enumeration-is-off-chain-not-an-entry-point)
