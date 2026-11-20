# Design: Withdraw to a Destination Address

**Status:** Accepted  
**Related Issues:** #457  
**Author:** Design Team  
**Date:** 2026-10-20

## Summary

This document evaluates whether to add a `withdraw_to(id, destination, amount)` entry point that allows the recipient to specify a different address to receive the withdrawn tokens, rather than always sending them to the stored `recipient` address. This would enable use cases such as withdrawing directly to a cold wallet, paying a third party, or routing payments through a smart contract without requiring an intermediate transfer.

## Motivation

Currently, `withdraw` and `withdraw_amount` always send tokens to the stream's `recipient` address. A recipient who wants funds delivered elsewhere must:

1. Call `withdraw` to receive tokens at their `recipient` address
2. Call `token.transfer` to move the tokens to the desired destination

This two-step workflow costs extra gas and requires the recipient to hold the tokens (even briefly) at an address they may not want to use for custody (e.g., a hot wallet when they prefer a cold wallet, or an EOA when they prefer a multisig).

A `withdraw_to` entry point would let the recipient specify the final destination in one call, saving a transaction and avoiding the intermediate holding step.

## Use Cases

### 1. Cold Wallet Withdrawals

A recipient who controls the stream via a hot wallet (for convenience) but wants funds custodied in a cold wallet can withdraw directly to the cold address without exposing the cold wallet's private key to the hot environment.

**Without `withdraw_to`:**
1. Hot wallet calls `withdraw(id)` → tokens go to hot wallet
2. Hot wallet calls `token.transfer(cold_wallet, amount)` → tokens move to cold wallet
3. Two transactions, tokens temporarily held in hot wallet

**With `withdraw_to`:**
1. Hot wallet calls `withdraw_to(id, cold_wallet, amount)` → tokens go directly to cold wallet
2. One transaction, cold wallet never touches the stream contract

### 2. Payment Splitting

A recipient who wants to split a withdrawal between multiple addresses (e.g., paying a vendor, a tax account, and a savings account) can do so in one call per destination, without routing everything through their own address first.

**Without `withdraw_to`:**
1. Recipient calls `withdraw(id)` → tokens go to recipient
2. Recipient calls `token.transfer(vendor, X)` → X tokens to vendor
3. Recipient calls `token.transfer(tax_account, Y)` → Y tokens to tax account
4. Remaining tokens stay in recipient's address

**With `withdraw_to`:**
1. Recipient calls `withdraw_to(id, vendor, X)` → X tokens to vendor
2. Recipient calls `withdraw_to(id, tax_account, Y)` → Y tokens to tax account
3. No intermediate holding, recipient's address is not involved

### 3. Smart Contract Recipients

A smart contract that is the recipient of a stream may want to forward withdrawn tokens to a different contract or EOA as part of its logic. `withdraw_to` would let the contract do this in one call, avoiding a `withdraw` followed by a `transfer` (which may itself trigger reentrancy concerns if the destination is another contract).

### 4. Third-Party Withdrawal Services

A service that manages withdrawals on behalf of multiple recipients (e.g., a payroll aggregator) can withdraw directly to the intended final destination without holding tokens in its own address, reducing custody risk and simplifying accounting.

## Proposed Design

### Entry Point

```rust
/// Withdraw vested tokens to a specified destination address.
///
/// Only the recipient may call this. Behaves like `withdraw_amount` but sends
/// the tokens to `destination` instead of the stream's stored `recipient`. The
/// stream's `withdrawn` counter is incremented by the amount transferred.
///
/// Returns the amount transferred.
///
/// # Authorization
///
/// - Requires authorization from the stream's `recipient` (not from `destination`).
///
/// # Validation
///
/// - `destination` cannot be the stream contract's own address (same rule as
///   `create_stream` participant checks).
/// - `destination` can be the same as `stream.recipient` (this is a valid but
///   redundant call; it behaves identically to `withdraw_amount`).
///
/// # Rejections
///
/// - [`StreamError::StreamNotFound`] if `id` does not exist in storage.
/// - [`StreamError::InvalidAmount`] if `amount` is zero or negative.
/// - [`StreamError::InsufficientBalance`] if `amount` exceeds the withdrawable balance.
/// - [`StreamError::InvalidParticipant`] if `destination` is the stream contract's own address.
///
/// # Examples
///
/// ```ignore
/// let amount = client.withdraw_to(&stream_id, &cold_wallet, &500);
/// ```
pub fn withdraw_to(
    env: Env,
    id: u64,
    destination: Address,
    amount: i128,
) -> Result<i128, StreamError>
```

### Authorization

**Only the stream's `recipient` authorizes the call.** The `destination` address is not asked to approve the transfer, consistent with how `token.transfer` works (the sender authorizes, the recipient receives without approval).

This means:
- The recipient can unilaterally send tokens to any address (except the contract itself).
- The destination address has no say in whether it receives the tokens (same as being the recipient in `create_stream` or the target of any `token.transfer`).

**Why not require `destination` to also authorize?**

Requiring dual authorization would break the single-transaction benefit (the recipient would have to coordinate with the destination to pre-approve), and it is inconsistent with the rest of the ecosystem (token transfers do not require recipient approval). The risk of "forced tokens" (sending tokens to an address that does not want them) is the same as creating a stream to an unwilling recipient, which is already permitted.

### Validation Order

Following the precedent of `create_stream` and `withdraw_amount`:

1. **Authorization:** `stream.recipient` must authorize.
2. **Stream existence:** `StreamNotFound` if `id` does not exist.
3. **Amount:** `InvalidAmount` if `amount <= 0`.
4. **Participant:** `InvalidParticipant` if `destination == env.current_contract_address()`.
5. **Balance:** `InsufficientBalance` if `amount > withdrawable(id)`.

**Why check `destination` before `InsufficientBalance`?**

Participant validation (step 4) is a precondition about the call's arguments, independent of the stream's state. Balance validation (step 5) depends on the current vested amount, which is time-dependent. Checking static argument validity before stateful balance checks is consistent with `create_stream`'s ordering (participants before schedule before capacity).

### State Changes

Identical to `withdraw_amount`:

1. `stream.withdrawn += amount`
2. Transfer `amount` of `stream.token` from `contract` to `destination` (not `stream.recipient`)
3. Publish `Withdrawn` event (see below)

The `stream.recipient` field is not altered. The `destination` is only used for the token transfer target; it has no persistent effect on the stream.

### Event

The existing `Withdrawn` event is reused, but with a semantic extension:

```rust
/// Emitted when tokens are withdrawn from a stream.
///
/// Topics:
/// - `recipient`: The stream's recipient (indexed, always the authorizing party).
///
/// Data:
/// - `stream_id`: The stream that was withdrawn from.
/// - `amount`: The amount of tokens withdrawn.
/// - `destination`: The address that received the tokens (added for `withdraw_to`).
///
#[contractevent]
pub struct Withdrawn {
    pub stream_id: u64,
    pub amount: i128,
    pub destination: Address,  // NEW: explicit destination field
}
```

**Changes to the Existing Event:**

- **Add `destination` field:** For `withdraw` and `withdraw_amount`, `destination == stream.recipient` (the tokens go to the recipient). For `withdraw_to`, `destination` is the address provided by the caller.
- **Keep `recipient` as the indexed topic:** Indexers filtering by "withdrawals by recipient X" should still use `recipient`, not `destination`. The `recipient` is the party who authorized the withdrawal, even if the tokens went elsewhere.

**Why not add a new event type (e.g., `WithdrawnTo`)?**

Indexers would have to track two separate event types for the same logical operation (a withdrawal). Extending the existing `Withdrawn` event with a `destination` field preserves backward compatibility: indexers that only care about "who withdrew" can still filter on `recipient`, and indexers that care about "where did the tokens go" can read the `destination` field.

**Backward Compatibility:**

Existing `Withdrawn` events (from before `withdraw_to` is added) do not have a `destination` field. Indexers reading historical events must handle this:

- For old events (pre-`withdraw_to`): assume `destination == recipient` if the field is absent.
- For new events (post-`withdraw_to`): read `destination` explicitly.

This is an **interface change** that would require a new contract deployment (see [docs/INTEGRATOR_OPERATIONS.md § Interface Stability Policy](../docs/INTEGRATOR_OPERATIONS.md#interface-stability-policy)).

### Interaction with Existing Entry Points

- **`withdraw(id)`:** Equivalent to `withdraw_to(id, stream.recipient, withdrawable(id))`. No change to existing behavior.
- **`withdraw_amount(id, amount)`:** Equivalent to `withdraw_to(id, stream.recipient, amount)`. No change to existing behavior.
- **`cancel(id)`:** The sender's refund still goes to `stream.sender`. `withdraw_to` does not affect cancellation.
- **Views:** `withdrawable`, `vested`, etc. are unaffected. They do not care where past withdrawals went, only the cumulative `withdrawn` amount.

## Risks and Considerations

### 1. Recipient Accidentally Sends to Wrong Address

If the recipient provides an incorrect `destination` address, the tokens are irretrievably sent to that address. This is the same risk as a user fat-fingering a `token.transfer`, but it is amplified because the recipient may not realize they are sending tokens to a third party (they might think `withdraw_to` is just a convenience function that still sends to themselves).

**Mitigation:**
- Clear documentation and UI warnings in client integrations.
- No on-chain mitigation is possible (the contract cannot validate whether the destination is "correct" for the recipient's intent).

### 2. Tokens Sent to an Inaccessible Address

If `destination` is a contract with no `transfer` or `withdraw` function, or an EOA whose private key is lost, the tokens are permanently stuck. This is the same risk as creating a stream to an inaccessible recipient, but it can happen on every withdrawal, not just once at creation.

**Mitigation:**
- The recipient is responsible for verifying the destination address before calling `withdraw_to`.
- Same risk as any token transfer in the ecosystem; not unique to streams.

### 3. Third-Party Indexers Confusion

An indexer filtering by "streams where address X is the recipient" would see `Withdrawn` events with `recipient == X` but `destination != X`. If the indexer is computing "X's received balance" by summing `Withdrawn` events, it would be wrong (the tokens did not go to X).

**Mitigation:**
- Indexers must read the `destination` field to track where tokens actually went.
- Indexers summing "recipient X's received balance" should filter on `destination == X`, not just `recipient == X`.
- This is an indexer logic change, and must be documented in [docs/INTEGRATOR_OPERATIONS.md](../docs/INTEGRATOR_OPERATIONS.md).

### 4. Reentrancy (If Destination Is a Contract)

If `destination` is a smart contract, the token transfer may trigger a callback to that contract (e.g., a token with transfer hooks, or a contract that implements a `receive` function). If that callback reenters the stream contract (e.g., calling `withdraw_to` again on the same stream), reentrancy could occur.

**Current Reentrancy Protection:**

The stream contract already updates `stream.withdrawn` **before** the token transfer (see `withdraw_with` in `contracts/stream/src/contract.rs`). This is the "checks-effects-interactions" pattern: state is updated before external calls, so a reentrant call sees the already-incremented `withdrawn` and cannot double-withdraw.

**With `withdraw_to`, this protection is preserved:**
1. `stream.withdrawn += amount` (state update)
2. `transfer(contract, destination, amount)` (external call, possibly reentrant)
3. Even if `destination` reenters via a hook, the `withdrawn` counter is already updated, so the second withdrawal sees reduced `withdrawable`.

**Additional Risk:**

If the token transfer calls back into the stream contract and that callback triggers another `withdraw_to` to a different address, the second withdrawal could send tokens to yet another destination. This is not a double-spend (the `withdrawn` counter still prevents over-withdrawal), but it could be surprising behavior (a single user action resulting in multiple withdrawals to multiple addresses).

**Mitigation:**
- The existing reentrancy guard is sufficient to prevent double-spend.
- Complex reentrant scenarios (e.g., `withdraw_to` triggering another `withdraw_to`) are the caller's responsibility to avoid. The contract guarantees the escrow invariant holds; it does not prevent the recipient from doing weird things with their own withdrawals.

### 5. Sender Cannot Control Destination

The sender has no say in where withdrawn tokens go. A sender who wants to restrict payments to a specific address (e.g., "only pay this recipient, no third parties") cannot enforce this on-chain with `withdraw_to` available.

**Is this a problem?**

No. The sender's agreement is with the recipient, not with where the recipient chooses to send their funds after receiving them. The sender escrows tokens for the recipient; what the recipient does with them (withdraw to themselves, withdraw to a cold wallet, withdraw to a vendor) is the recipient's business.

**Analogy:** A sender who pays an employee via direct deposit cannot control whether the employee immediately transfers their paycheck to a different account. The stream contract is the same: the sender's obligation is to make the funds available to the recipient, not to control where the recipient sends them afterward.

## Decision: Accept with Caveats

**Recommendation: Add `withdraw_to` as a new entry point.**

**Rationale:**

1. **Clear Utility:** The use cases (cold wallet withdrawals, payment splitting, smart contract routing) are legitimate and common.
2. **No New Security Risks:** The risks (wrong address, inaccessible destination, reentrancy) already exist in the broader ecosystem and are mitigated by existing patterns (checks-effects-interactions, user responsibility for addresses).
3. **Backward Compatible:** Existing `withdraw` and `withdraw_amount` are unaffected. Integrators can adopt `withdraw_to` incrementally.
4. **Interface Change Is Acceptable:** Adding a new entry point is an additive change. Extending the `Withdrawn` event is a breaking change, but it is one-time and well-defined.

**Caveats:**

1. **Clear Documentation Required:** The entry point's doc comment must warn that `destination` is not validated for correctness (it is the recipient's responsibility).
2. **Indexer Migration Required:** Indexers must be updated to handle the new `destination` field in `Withdrawn` events. This must be documented in [docs/INTEGRATOR_OPERATIONS.md](../docs/INTEGRATOR_OPERATIONS.md).
3. **UI Warning Recommended:** Client integrations should show a confirmation dialog when `destination != recipient`, making it clear the tokens will not go to the recipient's own address.

## Tests Required

### Unit / Integration Tests

1. **Happy path:**
   - Create a stream, call `withdraw_to(id, third_party, amount)`, verify `third_party` received the tokens and `recipient` did not.
   - Call `withdraw_to(id, recipient, amount)` (destination == recipient), verify it behaves identically to `withdraw_amount`.

2. **Authorization:**
   - Only the recipient can call `withdraw_to`. Non-recipient calling `withdraw_to` fails authorization (before `StreamNotFound` check).
   - The destination address is not required to authorize.

3. **Validation:**
   - `withdraw_to` with zero or negative amount fails with `InvalidAmount`.
   - `withdraw_to` with `destination == contract_address` fails with `InvalidParticipant`.
   - `withdraw_to` with `amount > withdrawable` fails with `InsufficientBalance`.
   - `withdraw_to` on a nonexistent stream fails with `StreamNotFound`.

4. **State consistency:**
   - After `withdraw_to`, `stream.withdrawn` is incremented by `amount`.
   - After `withdraw_to`, `destination` balance increases by `amount`.
   - After `withdraw_to`, `recipient` balance is unchanged (if `destination != recipient`).
   - The contract's token balance decreases by `amount`.

5. **Event correctness:**
   - `Withdrawn` event is published with correct `stream_id`, `amount`, `recipient`, and `destination`.
   - Event ordering: the event is published after the state update and transfer.
   - No event is published if the call is rejected.

6. **Interaction with `cancel`:**
   - `withdraw_to` to a third party, then cancel. The sender's refund is computed from `total_amount - withdrawn`, where `withdrawn` includes the `withdraw_to` amount.

7. **Reentrancy guard:**
   - If `destination` is a contract that reenters the stream contract via a token transfer hook, the second withdrawal sees the updated `withdrawn` counter and cannot double-withdraw.

### Property-Based Tests

1. **Escrow invariant after `withdraw_to`:**
   - For any stream and any withdrawal (to recipient or third party), `contract.token.balance == sum(stream.total_amount - stream.withdrawn)` across all streams.

2. **`withdraw_to(recipient, amount)` is equivalent to `withdraw_amount(amount)`:**
   - For any stream, calling `withdraw_to(id, stream.recipient, X)` should have the same effect as `withdraw_amount(id, X)`.

## Alternatives Considered

### Alt 1: No `withdraw_to` Feature

**Rejected:** The two-step workflow (`withdraw` then `transfer`) is inefficient and exposes tokens to the recipient's address even when they do not want to hold them there. The use cases are common enough to justify the feature.

### Alt 2: Require Destination Approval

**Rejected:** This would break the single-transaction benefit (the recipient would have to coordinate with the destination for pre-approval) and is inconsistent with how token transfers work elsewhere in the ecosystem.

### Alt 3: Add a New Event Type (`WithdrawnTo`)

**Rejected:** Splitting `Withdrawn` into two event types (`Withdrawn` for recipient, `WithdrawnTo` for third party) would complicate indexer logic (they would have to track both types for the same operation). Extending the existing event with a `destination` field is simpler.

### Alt 4: Only Allow `withdraw_to` for the Recipient (No Third Parties)

**Rejected:** This would defeat the purpose. The whole point of `withdraw_to` is to send tokens to a third party (or a different address controlled by the recipient, like a cold wallet). Restricting `destination == recipient` would make it identical to `withdraw_amount`.

### Alt 5: Validate Destination (e.g., Reject Contracts, Require Trustlines)

**Rejected:** The contract cannot know what a "valid" destination is for the recipient's intent. A contract recipient may be legitimate (e.g., a multisig). A missing trustline is the destination's responsibility, not the stream contract's. Over-validating would break legitimate use cases.

## Future Work

- **Batch `withdraw_to`:** A `withdraw_to_many(id, destinations: Vec<(Address, i128)>)` entry point that splits a withdrawal across multiple destinations in one call. Useful for payment splitting but adds complexity (what if one transfer fails?).
- **Destination Allowlist:** Letting the recipient pre-approve a set of destinations and reject `withdraw_to` to any address not on the list. Adds state and complexity; deferred until a concrete use case arises.

## Changelog

- **2026-11-20:** Accepted and implemented via #636.
- **2026-10-20:** Initial design proposal.

## References

- [README.md § How a stream works](../README.md#how-a-stream-works)
- [contracts/stream/src/contract.rs](../contracts/stream/src/contract.rs) — existing `withdraw` and `withdraw_amount` implementation
- [docs/INTEGRATOR_OPERATIONS.md](../docs/INTEGRATOR_OPERATIONS.md) — indexer requirements
- [THREAT_MODEL.md § Reentrancy](../THREAT_MODEL.md) — existing reentrancy analysis
