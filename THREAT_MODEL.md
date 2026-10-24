# Threat Model

This document describes the security properties and known limitations of the
`stream` contract. Read it before deciding how much value to lock.

## Design goals

The contract is intentionally minimal. It holds tokens on behalf of two
parties and enforces a linear release schedule. No administrator account
exists. No upgrade path is built in. The contract deployed to a given address
is the contract that will run for the lifetime of that address.

## No pause mechanism

**The contract has no pause, freeze, or emergency-stop function.**

There is no owner, admin, or multisig that can halt withdrawals, block a
sender's cancel, or prevent any other operation. Once a stream is created the
only parties that can affect it are the original sender (cancel) and the
original recipient (withdraw).

### Why this matters

Soroban contracts are immutable after deployment. If a bug is discovered in
the vesting logic, the token-transfer path, or any other part of the contract,
there is no mechanism to:

- stop new funds from being exposed to the vulnerability,
- freeze in-flight streams while a fix is prepared, or
- migrate locked tokens to a patched contract.

Every token locked in a stream is therefore exposed to any bug that exists in
the deployed bytecode for the full duration of that stream.

### Consequence for users

Anyone considering a long-duration stream — multi-month vesting schedules,
multi-year grants, subscription arrangements — should treat the contract's
audit status and the amount they are willing to lock as directly linked. A
stream that cannot be paused or migrated is a commitment whose risk profile
does not improve after the fact.

The sender's `cancel` function is the only unilateral escape hatch. It returns
the unvested portion to the sender, but it does not recover tokens that have
already vested to the recipient. If a bug affects the cancel path itself,
neither party has further recourse through the contract.

### Why no pause was added

A pause mechanism requires a privileged account. Introducing one would create
a new attack surface: the key that holds pause authority becomes a high-value
target, and its compromise would let an attacker freeze every stream on the
contract simultaneously. The design trades operational flexibility for the
removal of that privileged-key risk. This is an explicit choice, not an
oversight.

## Immutability

The contract bytecode is fixed at the Wasm hash recorded on-chain at
deployment. There is no `upgrade` entry point. A bug fix requires deploying a
new contract instance; existing streams do not move automatically.

## Authorization model

Every state-changing entry point is guarded by Soroban's `require_auth()`,
checked against the stream's own stored `sender` or `recipient` address — never
against a deployer, admin, or any other privileged account, because none
exists. Read-only views perform no authorization check at all: they alter no
state and move no tokens, so anyone can call them for any stream id.

| Entry point       | Kind      | Who may call it                | Enforced by                                                                                  |
| ------------------ | --------- | ------------------------------- | --------------------------------------------------------------------------------------------- |
| `create_stream`    | Mutating  | The `sender` named in the call  | `sender.require_auth()` (`contract.rs`, `create_stream`)                                      |
| `withdraw`         | Mutating  | The stream's `recipient`        | `stream.recipient.require_auth()` (`contract.rs`, `withdraw_with`, shared by `withdraw` and `withdraw_amount`) |
| `withdraw_amount`  | Mutating  | The stream's `recipient`        | `stream.recipient.require_auth()` (`contract.rs`, `withdraw_with`)                             |
| `cancel`           | Mutating  | The stream's `sender`           | `stream.sender.require_auth()` (`contract.rs`, `cancel`)                                      |
| `get_stream`       | View      | Anyone — no authorization       | none                                                                                           |
| `withdrawable`     | View      | Anyone — no authorization       | none                                                                                           |
| `vested`           | View      | Anyone — no authorization       | none                                                                                           |
| `locked`           | View      | Anyone — no authorization       | none                                                                                           |
| `progress`         | View      | Anyone — no authorization       | none                                                                                           |
| `status`           | View      | Anyone — no authorization       | none                                                                                           |
| `stream_count`     | View      | Anyone — no authorization       | none                                                                                           |

For `create_stream`, `sender` is whoever the caller names in the arguments —
authorization proves that address signed the transaction, not that it matches
any prior record, since no stream exists yet. For every other mutating entry
point, authorization is checked against the address already stored on that
specific stream record, so a `sender` or `recipient` from one stream has no
authority over any other stream. No entry point accepts an admin override,
and no account other than the two parties named at creation ever gains
authority over a stream.

## What a third party can observe

Everything a stream contains is public on the Stellar ledger. There is no
mechanism in this contract — and none available to a Soroban contract in
general — to keep a stream's participants, schedule, or amounts
confidential from anyone who chooses to look.

**What is observable, and how:**

- **Every stream field, for every stream id.** `get_stream` requires no
  authorization (see "Authorization model" above) and returns the full
  `Stream` record — `sender`, `recipient`, `token`, `total_amount`,
  `withdrawn`, `start_time`, `end_time`, `cliff_time`, and `cancelled` — to
  any caller. `stream_count` reports how many ids exist, so a third party can
  enumerate every stream ever created on a deployment by calling `get_stream`
  for every id from `0` to `stream_count - 1`.
- **Every event, with full participant and amount detail.** `Created` carries
  `sender`, `recipient`, `token`, `total_amount`, and the full schedule;
  `Withdrawn` carries `recipient` and `amount`; `Cancelled` carries `sender`,
  `recipient_amount`, and `sender_refund` (`contracts/stream/src/events.rs`).
  These are published on-chain for every call. Anyone running a Soroban RPC or
  Horizon query can reconstruct a stream's full history from them without
  ever calling the contract directly.
- **The underlying token movements themselves.** Each deposit, withdrawal,
  and refund is a real transfer on the streamed token's own contract (see
  the token interface note in [README.md](README.md)), visible to that
  token's own indexers and explorers the same way any other transfer is.
- **Transaction-level metadata** — who submitted each transaction, when, and
  with what fee — is part of the Stellar ledger itself, independent of
  anything this contract does.

**Amounts and addresses are not private.** The `sender` and `recipient`
addresses, and every amount this contract tracks — the total streamed, what
has vested, what has been withdrawn, and any refund — are plain, unencrypted
fields in public contract storage and public events. Nothing in this
contract or in the Soroban token interface it relies on offers a
confidential-amount or shielded-address feature.

**What this means for sensitive use cases.** A payroll run, a compensation
grant, or any other stream reveals who is paying whom, how much, and on what
schedule, from the moment it is created — to anyone willing to query the
contract or read the ledger, not only to the two parties involved. Before
streaming a salary or a sensitive payment, treat it as a published fact
rather than a private arrangement between sender and recipient. Using a
fresh address per stream reduces how easily that address links back to a
real-world identity elsewhere, but it does not hide the amount, the
schedule, or the existence of a payment relationship between whichever two
addresses are named on the stream.

## Invariants

These properties must hold after every successful call. A reviewer checking a
change to `contract.rs`, `vesting.rs`, or `storage.rs` should confirm each one
still holds rather than re-reading the whole contract from scratch.

- **A stream's escrow never exceeds its `total_amount`, and `withdrawn` never
  exceeds `total_amount`.** `create_stream` deposits exactly `total_amount`
  from the sender before the record is written (`contract.rs`,
  `create_stream`). Every later payout — `withdraw`, `withdraw_amount`, and
  the refund in `cancel` — is sized from `vesting::withdrawable_amount` or
  `vesting::settlement`, both of which derive from `vested_amount`, which is
  itself clamped to `[0, total_amount]` (`vesting.rs`). No path adds to
  `total_amount` or transfers more than the vested-but-unwithdrawn balance.
- **`withdrawn` only increases.** `withdraw_with` is the only writer of this
  field (`contract.rs`) and always does `stream.withdrawn += amount` with a
  non-negative `amount`; nothing ever decrements or resets it.
- **A cancelled stream never vests further, and cannot be cancelled again.**
  `cancel` freezes the schedule at the moment of cancellation — `total_amount`
  is cut down to the vested amount and `end_time` is set to `now`
  (`contract.rs`, `cancel`) — so `vested_amount` for any later `now` returns
  the same, already-frozen total. The leading `if stream.cancelled { return
  Err(StreamError::AlreadyCancelled) }` check makes a second `cancel` a no-op
  error rather than a second refund.
- **Only the sender may cancel; only the recipient may withdraw.** Enforced by
  `stream.sender.require_auth()` in `cancel` and
  `stream.recipient.require_auth()` in `withdraw_with` (the shared body of
  `withdraw` and `withdraw_amount`). No other entry point authorizes against
  a stream's stored `sender`/`recipient`.
- **A stream id is issued at most once and never reused.** `create_stream`
  reads `storage::stream_count`, uses that value as the new id, and advances
  the counter with a checked increment that returns
  `StreamError::StreamCountExhausted` instead of wrapping
  (`contract.rs`, `create_stream`; `storage.rs`).
- **`total_amount` is always positive and within `MAX_AMOUNT`.** Checked once,
  at creation, before any token moves (`contract.rs`, `create_stream`,
  `total_amount <= 0` / `total_amount > MAX_AMOUNT`). Nothing after creation
  can change `total_amount` except `cancel` reducing it downward to the
  vested amount.
- **`cliff_time` always falls within `[start_time, end_time]`, and
  `start_time < end_time`.** Both are checked at creation
  (`contract.rs`, `create_stream`) and never modified afterward except by
  `cancel`, which only ever moves them earlier or equal (`start_time.min(now)`,
  `cliff_time.min(now)`), never outside that relative order.

## Trust assumptions about the token contract

`create_stream` accepts a caller-supplied `token: Address` with no allowlist,
and every stream stores it: `withdraw`, `withdraw_amount`, and `cancel` all
invoke that same address later, on the caller's behalf, for the life of the
stream.

**What is assumed.** The token behaves as a conforming SEP-41 (or Stellar
Asset Contract) token: `transfer` moves exactly the requested amount between
the two accounts named, does not charge an undisclosed fee, does not itself
call back into this contract, and either succeeds or fails atomically with no
partial effect. The stream contract does not re-check the resulting balances
after a transfer; it trusts the token's own accounting.

### Decision: deposit verification by balance difference

**The contract does not verify deposits by reading the token balance before and after transfer.**

`create_stream` calls `TokenClient::transfer(&sender, &contract_address, &total_amount)` and immediately writes the stream record with that same `total_amount`, trusting the token to have moved exactly what was requested. The contract never calls `TokenClient::balance(&contract_address)` to confirm the actual deposit amount.

**What this protects against:** A fee-on-transfer token (one that deducts a percentage on every transfer, common on some EVM chains but not part of the SEP-41 standard) would deposit less than `total_amount` into the contract while the stream record claims the full amount. The shortfall would remain undetected until the final withdrawal fails due to insufficient contract balance, potentially months or years after creation. Reading the contract's balance before and after the transfer — `let before = token.balance(&contract_address); token.transfer(...); let after = token.balance(&contract_address); let actual = after - before;` — and using `actual` as the stream's `total_amount` instead of the caller's parameter would catch the discrepancy at creation.

**Cost of the check:** Two additional cross-contract `balance` calls per `create_stream` invocation. Each call incurs host function overhead, ledger entry read cost (the token contract's balance storage for this contract's address), and CPU time. The exact cost depends on the token implementation, but for a standard Stellar Asset Contract it adds roughly 0.2–0.5 ms and a small amount of ledger read fees. These are per-creation costs paid by the sender; they do not compound over the stream's lifetime.

**Decision rationale:** The contract rejects balance verification in favor of trusting the token interface. SEP-41 tokens do not charge fees on transfer — the standard specifies that `transfer(from, to, amount)` moves exactly `amount` or fails. A token that violates this by silently deducting a fee is non-conforming, and accommodating it would impose costs on every conforming-token user to protect against a scenario that should not exist in the Stellar Asset Contract ecosystem. Fee-on-transfer tokens are an EVM-specific hazard; Stellar Asset Contracts have no precedent for them, and the Soroban SDK's `TokenClient` interface does not expose a return value from `transfer` that would report the actual amount moved.

**Sender responsibility:** Choosing a token is entirely the sender's decision, made at `create_stream` time. A sender who streams a non-conforming token that silently deducts fees accepts the risk that the stream will fail on final withdrawal. The contract enforces no token allowlist and performs no runtime conformance check beyond confirming the address responds to a transfer call. Senders streaming significant value should verify the token is a genuine Stellar Asset Contract or a well-audited custom token before creating long-duration streams.

**Compatibility note:** Changing this decision — adding balance verification — would be a behavioral change affecting gas cost and execution time for every `create_stream` call. It would not break the interface signature, but it would alter the operational cost profile in a way integrators measuring and optimizing for gas costs would notice. The decision to trust the token is therefore locked in as part of the contract's deployed behavior.

**What a hostile or broken token can do.** Because the token is invoked with
the stream contract's own authority (the contract is the token's caller for
outgoing transfers), a token that violates these assumptions can affect that
stream's participants:

- **Silent short-transfer or an undisclosed fee.** A token that sends less
  than the requested amount without erroring would let a withdrawal report
  success while the recipient received less than the vesting math promised.
  Stream accounting (`withdrawn`, `total_amount`) is unaffected by the actual
  transferred amount, only by what was requested.
- **Reentrancy.** Soroban rejects direct re-entry into a contract already on
  the call stack. A token whose `transfer` attempts to call back into this
  contract (for example, by calling `withdraw` or `cancel` from a transfer
  hook) fails with a host `InvalidAction` error, aborting the entire
  transaction including the original operation that triggered the transfer.
  This is verified by the `token_reentrancy_during_withdraw_is_rejected` and
  `token_reentrancy_during_cancel_is_rejected` tests in
  `contracts/stream/src/tests/reentrancy.rs`. The protection is enforced by
  the Soroban host, not by this contract's own logic, so no state is left
  inconsistent: the re-entrant call never executes, and the original call is
  rolled back completely.
- **A token that never returns / always fails.** Every stream created with
  that token becomes permanently illiquid — no `withdraw`, `withdraw_amount`,
  or `cancel` can complete, because each aborts the whole call the same way
  any other transfer failure does (see below). The tokens already deposited
  stay in the contract, unreachable by either party, for as long as the
  chosen token remains broken.

**Who is responsible.** The contract enforces no allowlist and performs no
token-quality checks beyond confirming the address responds to a transfer
call. Choosing a token is entirely the sender's decision, made at
`create_stream` time; the contract offers no protection against a
malicious or defective one, and neither party can change the token on an
existing stream.

## What happens when a token transfer fails

`withdraw`, `withdraw_amount`, `cancel`, and `create_stream`'s initial deposit
all call the token's `transfer` through the plain (non-`try_`) client method
(`contract.rs`, the internal `transfer` helper and `create_stream`). If the
token contract returns an error or traps, that call itself traps, and Soroban
aborts the whole host invocation: **the entire transaction is rolled back, not
just the transfer.** Any storage write this contract made earlier in the same
call — the new stream record in `create_stream`, the incremented `withdrawn`
in `withdraw`/`withdraw_amount`, the frozen schedule in `cancel` — is undone
along with it, because a Soroban invocation only commits state if it returns
successfully. There is no path that leaves `withdrawn` incremented, or a
stream marked `cancelled`, while the corresponding transfer never happened.

**What the caller should do.** A failed transfer surfaces as a failed
transaction, not a typed `StreamError` — the failure is the token's, not this
contract's, so there is no `StreamError` variant for it. The caller should
inspect the failed transaction's diagnostic events to see which token call
failed and why (insufficient balance, no trustline, a frozen asset, or the
token contract's own error), resolve that condition, and retry the same
call: since nothing was committed, retrying is safe and not a double-spend.

**Token errors share StreamError's numeric range.** When a token transfer
fails, the error surfaces as `Error(Contract, #N)` with the token contract's
error number, which may collide with this contract's own `StreamError` codes.
For example, the Stellar Asset Contract's `BalanceError` is `10`, the same as
this contract's `AmountTooLarge`; its `OverflowError` is `12`, the same as
`StreamCountExhausted`; and its `TrustlineMissingError` is `13`, the same as
`InvalidParticipant`. A client that maps error numbers to messages using only
the `StreamError` table will show the wrong message for a token failure. To
distinguish them, check the failing contract's address in the diagnostic
events: if it matches this stream contract, use the `StreamError` table; if it
matches the token passed to `create_stream`, use that token's own error table.
See `contracts/stream/src/error.rs` for the full list of known collisions with
the Stellar Asset Contract.

## Storage lifetime and archival

Soroban storage entries expire on a ledger clock and are archived once their time to live runs out. The contract keeps two kinds of entry:

| Entry                  | Storage type | Holds                               |
| ---------------------- | ------------ | ----------------------------------- |
| `DataKey::Stream(id)`  | persistent   | one stream record                   |
| `DataKey::StreamCount` | instance     | the id to assign to the next stream |

Both are granted `ENTRY_TTL` (518,400 ledgers, roughly thirty days) and are extended back to that full window whenever accessed with fewer than `BUMP_THRESHOLD` (103,680 ledgers, roughly six days) remaining.

### Decision: instance TTL extension on withdraw and cancel

**`withdraw`, `withdraw_amount`, and `cancel` do not extend the instance TTL. Only `create_stream` extends it.**

The instance stores `DataKey::StreamCount`, the id counter that assigns every stream its unique id. `create_stream` calls `storage::extend_instance_ttl(&env)` after writing the new stream record ([`create_stream` in `contract.rs`](contracts/stream/src/contract.rs)), bringing the instance TTL back to the full `ENTRY_TTL` window if it has decayed below `BUMP_THRESHOLD`.

`withdraw`, `withdraw_amount`, and `cancel` touch only the individual stream's persistent entry (`DataKey::Stream(id)`), which is automatically bumped by Soroban when read or written. They do not call `extend_instance_ttl`. This behavior is pinned by the test [`withdraw_and_cancel_do_not_extend_the_instance_ttl` in `tests/storage.rs`](contracts/stream/src/tests/storage.rs).

**Failure mode if the instance expires:** If no stream is created for longer than `ENTRY_TTL` (518,400 ledgers, roughly thirty days) but `withdraw` and `cancel` continue to be called on existing streams, the instance TTL decays to zero and the instance is archived. Once archived:
- `create_stream` fails with a host storage error (not a `StreamError`), because it cannot read `StreamCount` from archived instance storage.
- `withdraw`, `withdraw_amount`, `cancel`, and all view calls continue to work for individual streams, because each stream is stored as a separate persistent entry with its own TTL, independent of the instance.
- The instance must be restored via an off-chain Soroban restore transaction before `create_stream` can succeed again.

**Quantifying the failure mode:** The scenario requires a deployment where streams are created, then no new stream is created for thirty days, but existing streams continue to be withdrawn from or cancelled. This is plausible for:
- A contract used for a fixed batch of streams (e.g., a one-time payroll run) where all streams are created on day 1, then over the next six months recipients withdraw periodically but no new streams are ever opened.
- A low-activity contract where stream creation happens sporadically (less than once per thirty days).

In both cases, the instance eventually expires. The first sender to attempt `create_stream` after that sees their transaction fail, and they or any other caller must submit a restore transaction to revive the instance. Existing streams are unaffected — recipients can still withdraw, and senders can still cancel.

**Cost of extending on every withdraw and cancel:** Each `extend_instance_ttl` call invokes `env.storage().instance().extend_ttl(ledgers_to_live, max_ttl)`, which reads the current instance TTL, compares it to `BUMP_THRESHOLD`, and if below the threshold, writes the new TTL back to host storage and charges the caller for the ledger write. The cost is a conditional ledger write, roughly 0.1–0.3 ms and a small storage fee.

For a high-frequency contract where hundreds or thousands of withdrawals occur per day, this cost compounds: every withdrawal below the bump threshold would pay to extend the instance, even though a single extension every few weeks (whenever a new stream is created) would suffice. The cost is per-call and paid by the caller (the recipient for `withdraw`, the sender for `cancel`).

**Decision rationale:** The instance TTL extension is **not added** to `withdraw`, `withdraw_amount`, and `cancel`. The failure mode (instance expiration after thirty days of no creation activity) is narrow, affecting only contracts with long gaps between stream creations, and the consequence is that the next `create_stream` requires a restore transaction — an operational inconvenience, not a loss of funds or a security breach. Existing streams remain fully functional; only new stream creation is blocked until the instance is restored.

The alternative — extending the instance on every withdrawal and cancellation — would impose a per-call cost on every active stream for the life of the contract to guard against a scenario that affects only idle or low-activity deployments. High-frequency contracts (payroll platforms, subscription services) would pay hundreds of times per month to extend an instance that `create_stream` already extends whenever it runs. The cost-benefit trade-off favors the current design: let `create_stream` handle instance extension, since it is the only operation that needs the instance to be live.

**Operational mitigation:** Deployments that expect long gaps between stream creations (more than thirty days) can:
1. Monitor the instance TTL using `env.storage().instance().get_ttl()` via a read-only view call or off-chain Soroban RPC query.
2. Submit a permissionless restore transaction before the instance expires, or immediately after expiration when the next `create_stream` is needed.
3. Create a dummy zero-value stream every few weeks to keep the instance alive, though this wastes a stream id and incurs the cost of a full `create_stream` call.

None of these mitigations are built into the contract. The responsibility for keeping the instance alive falls to the deployment operator, not to stream participants.

**Compatibility note:** Changing this decision would alter the operational cost profile for every high-frequency contract. It would not break the interface signature, but it would increase the gas cost and execution time for every withdrawal and cancellation call. The decision to extend only on `create_stream` is locked in as part of the contract's deployed behavior. The pinning test exists to prevent accidental changes during refactoring.

### Decision: permissionless stream TTL extension entry point

**A public `extend_stream_ttl(id)` entry point is not added to the contract.**

**The problem:** A stream that nobody touches for `ENTRY_TTL` (518,400 ledgers, roughly thirty days) is archived and must be restored before use. Any view call (`get_stream`, `withdrawable`, `vested`, etc.) or mutating call (`withdraw`, `cancel`) extends the stream's TTL, but if the recipient of a long vesting schedule (e.g., a two-year grant) never checks the stream for more than thirty days, it expires. Once expired, the next `withdraw` or `get_stream` call fails with a host storage error until someone submits an off-chain Soroban restore transaction. The recipient, or anyone else who cares about keeping the stream alive, would like a cheap way to extend its TTL without performing a full withdraw or view operation.

**Proposed entry point:**

```rust
pub fn extend_stream_ttl(env: Env, id: u64) -> Result<(), StreamError> {
    let stream = storage::get_stream(&env, id)?;
    storage::extend_stream_ttl(&env, id);
    Ok(())
}
```

This would be a permissionless call (no `require_auth()`) that anyone can invoke for any stream. It reads the stream to confirm it exists (returning `StreamError::StreamNotFound` if not), then calls `storage::extend_stream_ttl(&env, id)` to bump the stream's TTL back to `ENTRY_TTL` if it has decayed below `BUMP_THRESHOLD`. It performs no token transfer, no balance update, and no state change beyond the TTL extension itself.

**Cost:** One cross-contract call overhead (roughly 0.1–0.2 ms), one persistent storage read (`DataKey::Stream(id)`), and one conditional TTL write if below the bump threshold (the same cost as any other TTL bump, roughly 0.1–0.3 ms and a small storage fee). The caller pays the ledger read and write fees. For a single stream, this is cheaper than calling `get_stream` (which also reads the stream but then serializes and returns the full record to the caller) and far cheaper than a `withdraw` (which does all of that plus a token transfer).

**Abuse potential:** Because the entry point is permissionless, anyone can call it for any stream. The abuse scenarios are:

1. **Spam:** A malicious caller could invoke `extend_stream_ttl` for every stream id from `0` to `stream_count - 1` in a loop, forcing the contract to read and bump every stream's TTL whether it needs it or not. The caller pays the ledger fees, so this is a griefing attack rather than a free exploit, but it does consume network resources and could be used to bloat transaction history or exhaust a caller's fee budget in a denial-of-service pattern.

2. **Unwanted extension:** A recipient who wants their stream to expire (perhaps because they prefer it archived so the sender cannot cancel it, or because they want to avoid being tracked by periodic on-chain activity) cannot prevent a third party from keeping it alive. The permissionless nature removes recipient agency over the stream's archival state.

3. **No additional security risk:** The entry point cannot move tokens, cannot alter `withdrawn` or `total_amount`, and cannot change `sender`, `recipient`, or the schedule. It is purely a TTL management operation. An attacker calling it cannot drain, freeze, or hijack a stream; they can only keep it accessible longer than it would otherwise remain.

The first concern (spam) is the primary abuse vector. A determined griefer with sufficient fees could force-extend thousands of streams, but they gain nothing from it except making those streams stay alive longer, which is arguably neutral or even beneficial to the participants. The cost is paid by the attacker, not by the contract or its users.

**Benefit:** Recipients of long-vesting streams (multi-month or multi-year schedules) can keep their streams alive cheaply without having to withdraw early or query them solely for TTL-bumping purposes. A recipient who does not need funds for six months but wants the stream to remain accessible can call `extend_stream_ttl` once every few weeks at minimal cost instead of calling `withdrawable` or `get_stream` (which return data they do not need) or performing a zero-value withdrawal (which is not supported — `withdraw_amount(0)` returns `InvalidAmount`).

Third parties — such as an indexer service that tracks streams on behalf of users, or a grant administrator monitoring vesting schedules — can also call `extend_stream_ttl` to keep streams alive on behalf of participants who may not know the stream is about to expire. This is useful for non-technical recipients who do not monitor ledger state directly.

**Alternative: extend on every read call:** Instead of adding a dedicated entry point, modify `get_stream`, `withdrawable`, `vested`, `locked`, `progress`, and `status` to call `storage::extend_stream_ttl(&env, id)` before returning. This would make every read call bump the stream's TTL as a side effect, ensuring that any periodic query (e.g., an indexer checking `vested` every hour) keeps the stream alive without the need for a dedicated extension call.

Cost: The same conditional TTL write on every read call. The operational cost is identical to the proposed `extend_stream_ttl`, but the extension is implicit rather than explicit. The downside is that read calls would now perform a write as a side effect, which could surprise integrators who assume view calls are read-only and have no ledger-write cost.

**Decision rationale:** A permissionless `extend_stream_ttl(id)` entry point is **not added**. The benefit — allowing recipients and third parties to keep streams alive cheaply — is outweighed by the interface expansion and the abuse potential (spam and unwanted extension). The existing workaround is sufficient: any party who wants to keep a stream alive can call `get_stream(id)` or `withdrawable(id)` periodically (the TTL bump happens automatically as a side effect of reading the persistent entry), and the cost difference between that and a dedicated `extend_stream_ttl` call is small (a few milliseconds and a negligible fee for serializing the stream record).

Adding a new entry point increases the contract's API surface, requires additional testing (permissionless call semantics, interaction with archived streams, behavior across the full id range), and introduces the spam vector described above. The cost-benefit trade-off does not favor it. Recipients who hold long-vesting streams are expected to check them periodically — any view call is enough — and if a stream does expire, it can be restored via an off-chain transaction. The operational burden is minimal and does not justify a new entry point.

**If this decision were reversed:** A follow-up implementation would require:

1. **Signature:** `pub fn extend_stream_ttl(env: Env, id: u64) -> Result<(), StreamError>` as a new entry point in [`contract.rs`](contracts/stream/src/contract.rs), with no authorization check.

2. **Implementation:** Read the stream with `storage::get_stream(&env, id)?` to confirm it exists, then call `storage::extend_stream_ttl(&env, id)` to bump its TTL. Return `Ok(())` on success, `StreamError::StreamNotFound` if the id does not exist.

3. **Tests:**
   - Calling `extend_stream_ttl` for a valid stream id extends its TTL to `ENTRY_TTL` if it was below `BUMP_THRESHOLD`.
   - Calling `extend_stream_ttl` for a stream whose TTL is above `BUMP_THRESHOLD` does not change the TTL (deliberate no-op).
   - Calling `extend_stream_ttl` for a non-existent id returns `StreamError::StreamNotFound`.
   - Calling `extend_stream_ttl` does not alter `withdrawn`, `total_amount`, or any other stream field.
   - Permissionless: no authorization check, so any address can call it for any stream.

4. **Documentation:** Add the entry point to the contract interface table in the README, with "anyone" as the caller and "Extends the stream's TTL to prevent archival" as the description. Add a note to the storage lifetime section in the README and threat model explaining that the entry point exists as a convenience for keeping long-vesting streams alive without performing a withdrawal or full view call.

5. **Changelog:** Mark as a new entry point added in the version where it ships, with a compatibility note that it is permissionless and anyone can invoke it for any stream id.

## Out-of-scope risks

The following risks exist but are outside the scope of this contract:

- **Token contract bugs.** The streamed token is an external contract. A
  vulnerability in that contract can affect transfers regardless of stream
  contract correctness.
- **Stellar network-level events.** Ledger upgrades, validator behaviour, and
  protocol changes are outside this contract's control.
- **Key compromise.** If a sender's or recipient's private key is compromised,
  the attacker can cancel or drain the stream as that party.
- **Front-running.** Because Stellar transactions are publicly visible before
  inclusion, a recipient could theoretically race a cancel transaction, though
  the window is narrow and the vesting math caps what can be withdrawn.

## Summary

| Property | Value |
| --- | --- |
| Pause / emergency stop | **None** |
| Admin or owner account | **None** |
| Upgrade path | **None** |
| Per-stream escape hatch | Sender `cancel` (unvested portion only) |
| Contract immutability | Yes — Wasm hash fixed at deployment |
| Bug containment after deployment | Not possible without redeployment |
