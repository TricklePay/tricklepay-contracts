# Integrator Operations Guide

This guide collects the operational rules client authors need when integrating
with a deployed TricklePay stream contract.

## Redeployment Without Upgradeability

The stream contract is not upgradeable. A defect fix, SDK migration, or
interface change reaches users by deploying a new WASM and creating a new
contract address. There is no admin entry point that can replace code at an
existing address.

Existing streams remain on the deployment that created them. A redeployment
does not copy stream records, token balances, counters, or storage TTL state
from the old contract. Clients must therefore treat the contract id as part of
the stream identity:

```text
stream reference = (network, contract_id, stream_id)
```

Recipients claim from the old deployment by continuing to call `withdraw`,
`withdraw_amount`, and read-only views against that old `contract_id`. Senders
cancel old streams the same way: invoke `cancel` on the deployment that owns
the stream record. New streams should be opened on the new deployment only
after clients and indexers have been updated to recognize its address.

Recommended migration flow:

1. Deploy and verify the new contract address.
2. Publish the new address with the source commit and WASM hash.
3. Update clients to create new streams on the new address.
4. Keep old deployment metadata visible until every old stream is withdrawn,
   cancelled, or intentionally abandoned.
5. Index both old and new addresses during the overlap period.

Do not ask recipients to claim from the new deployment for an old stream. The
new deployment has no storage entry for that stream id and cannot transfer the
tokens locked in the old contract.

## Interface Stability Policy

Current stability expectation: the public interface is intended to be stable
for the lifetime of each deployment. Because deployments are immutable, a
breaking change is delivered as a new contract address rather than an in-place
upgrade.

The following are breaking interface changes:

- Removing, renaming, or changing the parameters or return type of an entry
  point.
- Changing the semantic meaning of an entry point while leaving its signature
  unchanged.
- Reordering, renaming, removing, or changing the type of event fields or
  topics.
- Reusing, renumbering, or changing the meaning of an error code.
- Changing storage semantics in a way that makes previously created streams
  unreadable or unclaimable.

Non-breaking changes include documentation updates, tests, internal refactors
that preserve emitted WASM interface output, and bug fixes that only reject
previously invalid or unsafe inputs when the changelog calls that out.

Every interface change must be communicated in `CHANGELOG.md`. Breaking
changes must identify the changed entry point, event, error code, or behavior,
and must tell integrators whether they need a new generated client, indexer
decoder, or contract address allowlist entry. Before deployment, compare the
interface with:

```bash
stellar contract info interface --wasm target/wasm32v1-none/release/tricklepay_stream.wasm --output rust
```

## Recommended Client Retry Behavior

Soroban submission failures are not all equivalent. A network timeout, dropped
HTTP connection, or unavailable RPC response does not prove the transaction
failed; it may have been accepted and applied after the client stopped waiting.

Clients must not blindly rebuild and resubmit a mutating call after an unknown
submission result. Retrying a fresh transaction can double-submit operations
such as `withdraw` once more balance vests, or create an unexpected second
stream if the first creation actually succeeded.

Recommended flow after a failed submission:

1. Preserve the transaction hash or operation intent locally.
2. Query the transaction status from the RPC or Horizon endpoint until it is
   known, expired, or definitively absent.
3. If the transaction succeeded, read the relevant contract view or event:
   `get_stream`, `withdrawable`, `stream_count`, or indexed events depending
   on the operation.
4. If the transaction failed with a contract error, surface that typed error to
   the user and do not retry automatically.
5. If the transaction expired or was never accepted, rebuild with a fresh
   sequence number and resubmit only after checking that the intended state
   change did not already happen.

For `create_stream`, verify whether a `Created` event exists for the sender,
recipient, token, amount, and schedule before resubmitting. For `withdraw` or
`withdraw_amount`, re-read `withdrawable` and the stream's `withdrawn` value
before deciding whether there is still anything to claim. For `cancel`, read
`status` and the `Cancelled` event before trying again.

## Indexer Consistency Requirements

Event topics and payloads are stable for the life of a deployment (see
Interface Stability Policy above), but that alone does not make an indexer
correct. Three things remain the indexer's own responsibility: **replayed
events**, **gaps in the event stream**, and **events it was not written to
understand**. Getting any of these wrong produces an index that looks fine
and is subtly wrong, with no local signal that anything is off.

### Handle repeated events idempotently

A closed ledger on a public Stellar network does not reorganize, but an
indexer can still observe the same event more than once — resubscribing from
an earlier cursor after a restart, replaying a checkpoint during backfill, or
reading the same ledger range from more than one RPC source. Never process an
event by blindly incrementing or appending.

- Key every processed event by a stable identity (ledger sequence,
  transaction, operation, and event index, or whatever equivalent the RPC
  exposes) and skip anything already recorded under that key before applying
  it.
- This matters most for `Withdrawn`, `ToppedUp`, and `Cancelled`: re-applying
  a `Withdrawn` event a second time would double the recorded cumulative
  withdrawn amount for a stream that never actually withdrew twice, and
  re-applying `ToppedUp.added_amount` would similarly overstate `total_amount`;
  that figure would then silently disagree with `get_stream`/`withdrawable`
  on-chain.
- `Created` happens to be idempotent to reapply, since it replaces a record
  with the same data — but dedupe it the same way regardless. Relying on
  per-event-type idempotence instead of one uniform rule is itself a source
  of bugs the moment a future deployment changes what a given event carries.

### Detect and backfill gaps

An indexer can miss events outright: downtime, starting to listen only after
the contract was already deployed, or querying an RPC endpoint whose
retention window for historical events has already passed. The contract has
no push mechanism to flag a missed event, so the indexer has to notice on its
own.

- Treat a `Withdrawn`, `ToppedUp`, or `Cancelled` event that names a stream id
  with no corresponding local `Created` record as a signal that the indexer's
  own history has a gap for that id — not as a contract inconsistency — and
  call `get_stream(id)` to recover that stream's fields directly from the
  contract rather than trying to reconstruct history that was never observed.
- Periodically compare the highest stream id the indexer has recorded against
  `stream_count()`. Any gap means ids exist on-chain with no local `Created`
  record, and should be backfilled with `get_stream` (or `get_streams` in
  batches of up to 20; see
  [Dashboard Guidance for get_streams](#dashboard-guidance-for-getstreams))
  for each missing id rather than treated as out of scope.
- `get_stream`, `withdrawable`, `vested`, `locked`, `progress`, and `status`
  require no authorization and always reflect current on-chain state
  regardless of how complete the local event history is (see
  [THREAT_MODEL.md § Authorization model](../THREAT_MODEL.md#authorization-model)).
  They are the correct recovery path for a gap, not a reason to mark a stream
  unindexable.

### Ignore, don't crash on, events it does not recognise

Within one deployment the event set is fixed — `Created`, `Withdrawn`,
`Cancelled`, and `ToppedUp`, with the fields documented in
[README.md § Events](../README.md#events) — but an indexer built to be
forward-compatible, or one scanning more broadly than a single contract id,
will still encounter things it was not written to decode:

- Events published by other contracts in the same ledger or transaction —
  most commonly the streamed token's own `transfer` event, since
  `create_stream`, `topup_stream`, `withdraw`, `withdraw_amount`,
  `withdraw_to`, `withdraw_many`, and `cancel` each trigger one or more. Filter
  by contract address before attempting to decode anything as a stream event.
- A future redeployment's event shape, since changing field order, a field's
  type, or the topic set is a breaking interface change delivered as a new
  contract address rather than an in-place change (see Interface Stability
  Policy above). An indexer tracking a migration across old and new
  deployments (see
  [Redeployment Without Upgradeability](#redeployment-without-upgradeability))
  must associate each event with the deployment whose shape it actually
  matches, not assume every address it tracks decodes identically.
- Skip and log an event your decoder does not recognise rather than crashing
  the process or forcing it into the nearest known shape. A silently
  mis-decoded event is worse than a loudly skipped one — it corrupts every
  derived figure below without leaving a trace.

### Store facts, derive figures

Not everything belongs in the indexer's own storage as a cached number. Split
what is stored from what is computed at read time:

- **Store** — facts an event sets, updates, or appends to: `sender`,
  `recipient`, `token`, `total_amount`, `start_time`, `end_time`, and
  `cliff_time` (from `Created`); updates to `total_amount` from `ToppedUp`
  (`new_total_amount`, since a top-up does **not** emit a new `Created` event);
  the running total of `amount` across every `Withdrawn` event for a given
  stream id (plus `destination` if displaying payout history); and, once seen, a
  `Cancelled` event's `recipient_amount`/`sender_refund`.
- **Derive at read time, and never cache as of some past moment:** `vested`,
  `withdrawable`, `locked`, `progress`, and `status`. Each is a pure function
  of the stored facts above and the current ledger time — the same formula
  the contract itself runs (see
  [README.md § Implementation details](../README.md#implementation-details)
  for the vesting arithmetic). A cached `withdrawable` value goes stale the
  instant ledger time moves past whenever it was computed, while recomputing
  it from the stored schedule costs nothing and is never wrong. The same
  applies to anything a client renders as "live" — a progress bar, a
  countdown, or a vested-amount display belongs on a value recomputed on
  every render, not one backfilled from the last event the indexer happened
  to see.

## Dashboard Guidance for get_streams

User-facing dashboards and portal applications frequently display streams in lists: active streams belonging to an account, outgoing payroll runs, or recent transactions. Querying streams individually using `get_stream(id)` introduces an N+1 query problem, requiring one network round-trip per stream and quickly exhausting RPC rate limits.

The contract provides `get_streams(ids: Vec<u64>) -> Result<Vec<Option<Stream>>, StreamError>` to batch up to 20 stream lookups into a single call.

### Chunking and Batching Requests

`get_streams` enforces a hard limit of `MAX_BATCH_SIZE = 20` stream IDs per request. When a dashboard displays more than 20 streams, the client must chunk the list of IDs into slices of at most 20 IDs.

The return value is a vector of `Option<Stream>` of the exact same length and in the exact same order as the requested `ids`:
- A found stream is returned as `Some(stream)`.
- An unknown, non-existent, or archived stream is returned as `None` at that position. The call does **not** fail if an ID is missing.

#### Worked Example: Batching, Chunking, and Splitting on Error

The following client helper demonstrates chunking an arbitrary list of stream IDs into batches of at most 20, querying `get_streams`, handling `None` entries, and recovering from `BatchTooLarge` by subdividing:

```typescript
import { Stream, StreamError } from "@tricklepay/sdk";

const MAX_BATCH_SIZE = 20;

/**
 * Fetch multiple streams in batches of up to 20.
 *
 * If a batch returns StreamError.BatchTooLarge (code 14), it splits
 * the batch in half and retries each slice.
 */
async function fetchStreamsBatched(
  client: StreamClient,
  streamIds: bigint[]
): Promise<Map<bigint, Stream | null>> {
  const results = new Map<bigint, Stream | null>();

  // Helper to process a single slice of IDs
  async function fetchChunk(chunk: bigint[]): Promise<void> {
    if (chunk.length === 0) return;

    try {
      const records = await client.get_streams({ ids: chunk });
      for (let i = 0; i < chunk.length; i++) {
        const id = chunk[i];
        const record = records[i] ?? null;
        results.set(id, record);
      }
    } catch (err: any) {
      // If code 14 (BatchTooLarge) is encountered, split and retry
      if (err?.code === 14 || err?.name === "BatchTooLarge") {
        if (chunk.length === 1) {
          throw new Error(`Single stream ID ${chunk[0]} returned BatchTooLarge`);
        }
        const mid = Math.floor(chunk.length / 2);
        await fetchChunk(chunk.slice(0, mid));
        await fetchChunk(chunk.slice(mid));
        return;
      }
      throw err;
    }
  }

  // Chunk all requested stream IDs into slices of at most 20
  for (let i = 0; i < streamIds.length; i += MAX_BATCH_SIZE) {
    const chunk = streamIds.slice(i, i + MAX_BATCH_SIZE);
    await fetchChunk(chunk);
  }

  return results;
}
```

### Failure Modes and Retry Behavior

1. **`StreamError::BatchTooLarge` (code `14`)**:
   - **Trigger:** Calling `get_streams` with `ids.len() > 20`.
   - **Retry behavior:** Do **not** retry with the same argument. The failure is deterministic and immediate. Subdivide the batch into smaller chunks (for example, halve the chunk size to 10) and submit each half separately.
2. **Missing or archived streams (`None` entries)**:
   - **Trigger:** Passing a stream ID that does not exist in storage or whose persistent storage entry expired past its TTL window.
   - **Retry behavior:** Do not treat `None` as a fatal RPC or batch error. The client should filter out `None` values or display an indicator (e.g., "Archived or not found"). If an active stream expired, it can be restored on-chain via a Soroban restore transaction before re-querying.
3. **Transient network or RPC errors**:
   - **Trigger:** Dropped HTTP connection, RPC timeout, or rate-limit responses (HTTP 429 / 5xx).
   - **Retry behavior:** `get_streams` is a read-only view that does not write contract state, alter balances, or extend storage TTLs. It is completely safe to retry immediately using exponential backoff and jitter. Unlike mutating entry points (`withdraw`, `create_stream`), retrying a view cannot produce duplicate side effects.
4. **Storage TTL note**:
   - `get_streams` reads entries with `storage::get_stream_readonly`, which does **not** extend the stream's TTL (`ENTRY_TTL`). Dashboards reading streams periodically cannot rely on `get_streams` to prevent stream archival; participants or maintainers must use mutating transactions or single `get_stream(id)` calls to bump storage TTL.

### Computing Derived Figures Client-Side

Each returned `Stream` record contains only the immutable schedule facts and current withdrawal counter: `total_amount`, `withdrawn`, `start_time`, `end_time`, `cliff_time`, and `cancelled`.

Dashboards should **not** invoke individual view entry points (`vested`, `withdrawable`, `locked`, `progress`, `status`) over RPC for every stream in the batch. Doing so would turn a single batch call into $5 \times N$ additional network round-trips. Instead, compute all live figures client-side from the returned `Stream` record and the current ledger timestamp (`now = env.ledger().timestamp()` or local clock synced to network ledger close time).

The formulas match the contract's on-chain arithmetic exactly:

- **Vested amount (`vested`):**
  Linear vesting with integer division truncation (see [README.md § Rounding direction](../README.md#rounding-direction) and [README.md § Integer rounding](../README.md#integer-rounding)):
  - If $\text{now} < \text{cliff\_time}$ or $\text{now} < \text{start\_time}$, $\text{vested} = 0$.
  - If $\text{now} \ge \text{end\_time}$, $\text{vested} = \text{total\_amount}$.
  - Otherwise, $\text{vested} = \lfloor \frac{\text{total\_amount} \times (\text{now} - \text{start\_time})}{\text{end\_time} - \text{start\_time}} \rfloor$.
  Implemented in [`contracts/stream/src/vesting.rs`](../contracts/stream/src/vesting.rs).

- **Withdrawable balance (`withdrawable`):**
  $$\text{withdrawable} = \max(\text{vested} - \text{withdrawn}, 0)$$
  Implemented in [`contracts/stream/src/vesting.rs`](../contracts/stream/src/vesting.rs).

- **Locked amount (`locked`):**
  The unvested remainder of the schedule (see [README.md § locked: a precise figure, not a synonym for "still escrowed"](../README.md#locked-a-precise-figure-not-a-synonym-for-still-escrowed)):
  $$\text{locked} = \max(\text{total\_amount} - \text{vested}, 0)$$

- **Vesting progress in basis points (`progress`):**
  $$\text{progress} = \lfloor \frac{\text{vested} \times 10\,000}{\text{total\_amount}} \rfloor \quad (\text{range } [0, 10\,000])$$

- **Lifecycle status (`status`):**
  Following the precedence in [`contracts/stream/src/status.rs`](../contracts/stream/src/status.rs):
  - If $\text{stream.cancelled}$ is true, status is $\text{Cancelled}$.
  - Else if $\text{now} < \text{start\_time}$, status is $\text{Pending}$.
  - Else if $\text{now} \ge \text{end\_time}$, status is $\text{Completed}$.
  - Otherwise, status is $\text{Streaming}$.

Recomputing these values client-side costs virtually nothing, allows smooth real-time progress animations, and keeps the dashboard resilient against network latency.

## Top-Up, Destination Withdrawal, and Batch Withdrawal Guidance

### Keeping `total_amount` Current from `ToppedUp` Events (`topup_stream`)

Calling `topup_stream(id, added_amount)` increases `stream.total_amount` on-chain while preserving `start_time`, `cliff_time`, and `end_time`, and emits [`ToppedUp`](../contracts/stream/src/events.rs#L49) — it does **not** emit a new `Created` event.

- **Topics & fields:** `ToppedUp` indexes `sender` as a topic and carries `stream_id`, `added_amount`, `new_total_amount`, and `new_end_time` in data.
- **Updating `total_amount`:** On a deduplicated `ToppedUp` event, update the stored stream record's `total_amount` to `new_total_amount` (and `end_time` to `new_end_time`, which equals the existing `end_time` under the current rate-increase semantic). Do not reset `withdrawn`.
- **Retroactive vesting jump:** Once `total_amount` increases, client-side `vested`, `withdrawable`, and `locked` calculations must recompute from the new `total_amount`; for a stream already past `start_time` (and `cliff_time`, if set), `vested` and `withdrawable` jump immediately without time advancing.
- **Backfilling gaps:** If `ToppedUp` arrives for a `stream_id` with no local `Created` record, call `get_stream(stream_id)` to backfill the current on-chain state (which already reflects the post-top-up `total_amount`).

### Reading `destination` on `Withdrawn` Events (`withdraw_to`)

[`Withdrawn`](../contracts/stream/src/events.rs#L26) indexes `recipient` as a topic and carries `id`, `amount`, and `destination` in data:

- **Standard withdrawals (`withdraw`, `withdraw_amount`, `withdraw_many`):** `destination` is equal to `stream.recipient`.
- **Destination withdrawals (`withdraw_to(id, destination, amount)`):** `destination` is the address that actually received the token transfer (for example, a cold wallet or third-party address), while the `recipient` topic remains `stream.recipient` (the party that authorized the call).
- **How indexers and dashboards should read it:** Always attribute `amount` to the stream's `withdrawn` total using `id` (and `recipient` for stream ownership), and read `destination` as the payout target for transaction history views and token-transfer reconciliation.

### Batch Withdrawals (`withdraw_many`)

`withdraw_many(ids: Vec<u64>) -> Result<Vec<i128>, StreamError>` lets a caller claim all withdrawable balances across multiple streams in a single transaction, returning a `Vec<i128>` of amounts paid in the same order as `ids`:

- **Batch limit:** Enforces `MAX_BATCH_SIZE = 20` stream IDs per call; passing more than 20 IDs fails with `StreamError::BatchTooLarge`. Chunk larger lists into slices of at most 20 IDs.
- **One unknown ID fails the entire call:** Unlike `get_streams` (which returns `None` for missing IDs), `withdraw_many` is atomic — if any ID in `ids` does not exist, the whole call fails with `StreamError::StreamNotFound` and no transfers or state updates occur.
- **Authorization & zero-payout streams:** Every distinct `recipient` across `ids` must authorize the call. Streams in the batch with `0` withdrawable balance are skipped (recording `0` in their output slot and emitting no `Withdrawn` event); if no stream in the batch pays out a positive amount, the whole call fails with `StreamError::NothingToWithdraw`.
- **Events:** Emits **one `Withdrawn` event per paid stream** (each with `destination == stream.recipient`), paired with one token transfer per paid stream.

## Withdrawing Before Anything Has Vested

`withdraw` is not a no-op when there is nothing to claim: it returns
`StreamError::NothingToWithdraw` (interface code `7`) rather than silently
succeeding or transferring zero tokens. This happens whenever the vested
amount minus what was already withdrawn is zero or less, including:

- Calling `withdraw` before `start_time`, or before `cliff_time` on a stream
  that has one.
- Calling `withdraw` again immediately after a previous withdrawal has
  already claimed everything vested so far.

`withdraw_amount` behaves differently in this situation: because the caller
names an amount, a positive request against zero available balance fails
with `StreamError::InsufficientBalance` (code `8`) instead of
`NothingToWithdraw`. Only the zero-argument `withdraw` entry point returns
`NothingToWithdraw`.

### How a client should present this

This is an expected state, not a failure. A recipient who opens a stream
before the cliff, or right after draining it, will hit this every time. Treat
`NothingToWithdraw` as information to surface in the UI ("Nothing available
to claim yet" / "Already withdrawn everything that has vested"), not as an
error toast, retry prompt, or logged failure. Do not retry the call
automatically; retrying without the underlying state changing will return the
same error.

### How to check beforehand

Call the `withdrawable` view before submitting a `withdraw` transaction:

```ignore
let available = client.withdrawable(&stream_id);
if available > 0 {
    client.withdraw(&stream_id);
}
```

`withdrawable` returns `vested_amount - withdrawn` for the stream, clamped to
a minimum of zero (see [`vesting::withdrawable_amount`]). A result of `0`
means a `withdraw` call would fail with `NothingToWithdraw` right now, so the
client can disable the withdraw action, hide it, or show the informational
state instead of attempting the call and parsing an error. Because vesting is
time-based, `withdrawable` can change between the check and the submitted
transaction landing; treat a `NothingToWithdraw` error that still occurs as
the authoritative answer rather than a bug in the pre-check.

## Token Due Diligence Checklist for Senders

**The stream contract performs no token validation.** It accepts any address
the sender provides at `create_stream` time and invokes that address for every
transfer over the life of the stream. A defective or malicious token can lock
funds, short-transfer amounts, or behave unpredictably. Choosing a token is
entirely the sender's responsibility. **This checklist tells you what to verify
before creating a stream.** Skipping these checks can result in permanent loss
of funds or a stream that cannot be withdrawn or cancelled.

### 1. The token conforms to SEP-41 (Soroban token interface)

The contract expects every token to implement the standard `transfer(from: Address, to: Address, amount: i128)` function with the semantics defined in [SEP-41](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md):

- `transfer` moves exactly the requested `amount` from `from` to `to`.
- It returns success (`()`) if the transfer completed, or an error if it did not.
- It does not perform a partial transfer, charge an undisclosed fee, or succeed while moving less than `amount`.

**Why this matters:** The contract does not re-check balances after a transfer. It trusts the token's own accounting. A token that silently transfers less than requested will cause the stream's internal `withdrawn` counter to disagree with the recipient's actual received balance. See [THREAT_MODEL.md § Trust assumptions about the token contract](../THREAT_MODEL.md#trust-assumptions-about-the-token-contract) for the full list of what can go wrong.

**How to check:**
- Verify the token contract source or WASM is published and audited.
- Confirm it implements SEP-41 or is a Stellar Asset Contract (SAC) deployed through the standard issuer flow.
- If the token is custom, test a transfer with a known amount and verify the recipient's balance increased by exactly that amount.

### 2. The token charges no transfer fee

Some token contracts deduct a fee from every transfer — for example, a 1% cut that goes to a treasury or burn address. The stream contract is not aware of fees and does not adjust its accounting for them.

**Why this matters:** If the token deducts a fee, the recipient will receive less than the contract's vesting math promised, but `withdrawn` will still be incremented by the full requested amount. Over time, the stream's internal ledger will drift from the recipient's actual received balance, and the final withdrawal may fail because the contract believes it has already sent more than the token actually transferred. See [THREAT_MODEL.md § What a hostile or broken token can do](../THREAT_MODEL.md#trust-assumptions-about-the-token-contract).

**How to check:**
- Read the token's documentation or source code. Look for any mention of "transfer fee," "transaction tax," "burn on transfer," or similar.
- Test: send a known amount between two accounts you control and verify the recipient received exactly that amount with no deduction.

### 3. The token has no transfer hooks that call back into the stream contract

Some token contracts invoke hooks or callbacks on the sender, recipient, or a registered observer as part of a transfer. If a token calls back into the stream contract from within the transfer — for example, from a recipient-side hook triggered by a `withdraw` — the contract may be reentered while its own state update is still in flight.

**Why this matters:** Reentrancy is a known vulnerability class in smart contracts. The stream contract's design minimizes reentrancy risk by updating state (`withdrawn`, `total_amount`, `cancelled`) before most transfers (see [THREAT_MODEL.md § What a hostile or broken token can do](../THREAT_MODEL.md#trust-assumptions-about-the-token-contract)), but a token that reenters can still interleave operations in ways the test suite does not exercise. The safest choice is a token with no hooks at all.

**How to check:**
- Read the token's source code. Look for any "hook," "callback," "observer," or "on_transfer" mechanism that invokes an external contract during a transfer.
- If the token is a Stellar Asset Contract (SAC), it has no hooks — SACs follow the standard SEP-41 interface with no extensions.
- If the token is custom and does have hooks, verify that none of the registered hook targets are the stream contract itself, and that the token's hook logic cannot be pointed at an arbitrary address by an attacker.

### 4. The issuer cannot freeze, claw back, or unilaterally mint tokens to the stream contract

Some tokens grant the issuer (or an admin account) the ability to freeze an account's balance, revoke tokens that have already been transferred, or mint new tokens into any address without authorization. If the issuer can do any of these to the stream contract's address, they can drain the contract, block withdrawals, or inflate the balance in ways the contract does not account for.

**Why this matters:** If the issuer freezes the stream contract, every `withdraw`, `withdraw_amount`, and `cancel` that tries to transfer tokens out will fail, and all funds in the contract become permanently illiquid. If the issuer can claw back tokens the sender already deposited, the contract's internal accounting will still reflect the original `total_amount`, but the actual balance will be lower, and withdrawals will eventually fail when the contract tries to send more than it holds. See [THREAT_MODEL.md § Trust assumptions about the token contract](../THREAT_MODEL.md#trust-assumptions-about-the-token-contract).

**How to check:**
- For Stellar native assets and SACs: check the issuer's flags on Stellar's ledger. If `AUTH_IMMUTABLE` is not set, or if `AUTH_REVOCABLE` or `AUTH_CLAWBACK_ENABLED` is set, the issuer retains control. **Do not stream such a token unless you fully trust the issuer.**
- For custom Soroban tokens: read the source. Look for any entry point that lets an admin freeze accounts, revoke balances, or mint without the recipient's authorization. If such functions exist, treat the token as unsuitable for streaming unless you control that admin key yourself.

### 5. All required trustlines are in place

On Stellar, holding a non-native asset requires the recipient to establish a trustline to the issuer before they can receive it. If the recipient has no trustline for the streamed token, every `withdraw` and `withdraw_amount` will fail with a token-side error (typically `TrustlineMissingError`), and the stream will be effectively unusable until the recipient creates the trustline.

**Why this matters:** A stream whose recipient cannot receive transfers is a locked stream. The sender can still `cancel` and recover the unvested portion, but the vested amount stays in the contract, unclaimable by the recipient, until they add the trustline. The contract has no mechanism to notify the recipient or enforce trustline setup beforehand. See [THREAT_MODEL.md § What happens when a token transfer fails](../THREAT_MODEL.md#what-happens-when-a-token-transfer-fails).

**How to check:**
- If the token is a Stellar asset or SAC, query the recipient's account from Horizon or an RPC and verify a trustline exists for the asset being streamed.
- If the token is a custom Soroban token that does not use Stellar's trustline model, check that the recipient is registered with the token contract (if registration is required) or that the token allows transfers to any address.
- Confirm this before calling `create_stream`. If the recipient does not have the trustline, ask them to establish it first, or use a different recipient address that already has it.

### Summary: skip these checks at your own risk

The stream contract does not enforce any of the above. It calls the token you give it and trusts that the token behaves as described in SEP-41. **If the token does not, the contract offers no protection, and neither party has recourse through the contract's interface.** The threat model ([THREAT_MODEL.md § Trust assumptions about the token contract](../THREAT_MODEL.md#trust-assumptions-about-the-token-contract)) documents every failure mode a non-conforming token can cause. This checklist is the actionable version: verify these five things before you stream, or accept the risk that one of them is violated and the stream becomes illiquid, short-changes the recipient, or behaves unpredictably.

**When in doubt:** Use a Stellar Asset Contract for a well-known Stellar asset (XLM, USDC, a stablecoin from a recognized issuer) rather than a custom or unknown token. SACs are standardized, have no hooks, and their issuer controls are visible on-chain. Custom tokens are only as safe as their implementation and the deployer's intentions, and the contract cannot tell the difference.

## Distinguishing Stream Errors from Token Errors in Failed Transactions

When a transaction involving the stream contract fails, the error surfaces as a
contract error number — for example, `Error(Contract, #10)`. Because the stream
contract invokes the token contract for every transfer operation (`create_stream`,
`withdraw`, `withdraw_amount`, `cancel`), a failure could come from either
contract. **Both contracts use overlapping error code ranges,** so the numeric
code alone does not tell you which contract raised it.

For example:
- `StreamError::AmountTooLarge` is code `10` in the stream contract.
- The Stellar Asset Contract's `BalanceError` is also code `10`.
- `StreamError::StreamCountExhausted` is code `12`; the SAC's `OverflowError` is also `12`.
- `StreamError::InvalidParticipant` is code `13`; the SAC's `TrustlineMissingError` is also `13`.

A client that maps error numbers to messages using only the `StreamError` table
will show the wrong message for a token failure, and vice versa. **To
distinguish them, check which contract address appears in the diagnostic events
for the failed call.**

### How diagnostic events work

When a Soroban contract call fails, the Stellar RPC includes diagnostic events
in the simulation or transaction result. These events contain structured
metadata about what happened during execution, including:
- Which contract was invoked at each step.
- The function name that was called.
- The error that was raised, along with the contract address that raised it.

**The key piece of information is the contract address in the error event.** If
it matches the stream contract's address, the error is a `StreamError`. If it
matches the token address passed to `create_stream`, the error is from the
token contract, and you must use that token's error table to decode it.

### Worked example 1: A stream error (`StreamError::NothingToWithdraw`)

**Scenario:** A recipient calls `withdraw` on a stream before `start_time`, when
nothing has vested yet.

**What the transaction looks like:**
```json
{
  "status": "FAILED",
  "error": "Error(Contract, #7)"
}
```

**Diagnostic events excerpt:**
```json
{
  "events": [
    {
      "type": "diagnostic",
      "contract": "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",  // stream contract address
      "topics": [
        "error",
        "withdraw"
      ],
      "data": {
        "error": "Error(Contract, #7)"
      }
    }
  ]
}
```

**How to interpret:** The `contract` field in the error event is the stream
contract's address. Error code `7` is `StreamError::NothingToWithdraw`. The
message for the user is: "Nothing available to withdraw yet" (see
[Withdrawing Before Anything Has Vested](#withdrawing-before-anything-has-vested)).
The failure came from the stream contract's own logic — no token call was
involved.

### Worked example 2: A token error (Stellar Asset Contract `TrustlineMissingError`)

**Scenario:** A sender calls `withdraw` on a stream, but the recipient does not
have a trustline for the streamed asset.

**What the transaction looks like:**
```json
{
  "status": "FAILED",
  "error": "Error(Contract, #13)"
}
```

**Diagnostic events excerpt:**
```json
{
  "events": [
    {
      "type": "diagnostic",
      "contract": "CBQHNAXSI55GX2GN6D67GK7BHVPSLJUGZQEU7WJ5LKR5PNUCGLIMAO4K",  // token contract address
      "topics": [
        "error",
        "transfer"
      ],
      "data": {
        "error": "Error(Contract, #13)"
      }
    }
  ]
}
```

**How to interpret:** The `contract` field in the error event is the token
contract's address — the `token: Address` that was passed to `create_stream`,
not the stream contract itself. Error code `13` from a Stellar Asset Contract
is `TrustlineMissingError`. The message for the user is: "The recipient does
not have a trustline for this asset. They must establish a trustline before
withdrawing." (See [Token Due Diligence Checklist § All required trustlines are
in place](#5-all-required-trustlines-are-in-place).) The failure came from the
token's `transfer` call, which the stream contract invoked but did not cause.

If code `13` had instead come from the stream contract's address, it would be
`StreamError::InvalidParticipant` (a different error), and the message would be
completely wrong.

### When diagnostic events are available

Diagnostic events are included in:
- **Transaction simulation results** when you call `simulateTransaction` on a
  Stellar RPC before submitting. This is the recommended way to preview what
  will happen and catch errors before paying fees.
- **Failed transaction results** after a transaction has been submitted to the
  network and applied. The transaction result includes the same diagnostic
  event structure.

Diagnostic events are **not available** in:
- Successful transactions by default (they may optionally be included if the
  RPC is configured to emit them, but most production RPCs do not).
- Transactions that fail during pre-flight validation (malformed XDR, invalid
  signature, insufficient balance to pay the base fee) before they reach
  contract execution. These fail at the Stellar protocol layer, not inside a
  contract, so no diagnostic events are generated.

If a transaction succeeds, you do not need to parse error events — the result
carries the returned value, and any `Created`, `Withdrawn`, or `Cancelled`
event the contract published. If it fails during pre-flight, the error is a
Stellar protocol error (see [Stellar's transaction result
codes](https://developers.stellar.org/docs/fundamentals-and-concepts/stellar-data-structures/operations-and-transactions#result-codes)),
not a contract error.

### Recommended client flow

1. **Before submitting:** Call `simulateTransaction` and check the result. If
   the simulation fails, inspect the diagnostic events for the failing
   contract's address and error code. Decode the error using the correct
   contract's error table (stream contract or token contract) and show the user
   a specific, actionable message before they pay to submit a transaction that
   will fail.

2. **After a failed submission:** Read the transaction result's diagnostic
   events the same way. Match the failing contract address to either the stream
   or token, decode the error, and present it to the user with context ("This
   failed because the token transfer could not complete" vs. "This failed
   because the stream contract rejected the operation").

3. **Collision-safe error decoding:** Never map an error code to a message
   without first checking which contract raised it. Maintain separate error
   tables for the stream contract and for every token a stream might use (or at
   least for the Stellar Asset Contract, which is the most common). If you
   encounter an error from an unknown contract, show the raw code and contract
   address rather than guessing.

### Related references

- `contracts/stream/src/error.rs` — full list of `StreamError` codes, including
  notes on which ones collide with Stellar Asset Contract error numbers.
- [THREAT_MODEL.md § What happens when a token transfer
  fails](../THREAT_MODEL.md#what-happens-when-a-token-transfer-fails) — explains
  that a token transfer failure aborts the entire transaction, not just the
  transfer, and how to recover.

## Maximum Practical Stream Duration

The type system permits `u64` Unix-second timestamps, but the practical maximum
stream duration is much shorter than that. Treat **30 days without any stream
access** as the operational limit imposed by storage rent, and treat streams
longer than 30 days as safe only if a keeper, indexer, wallet, or user action
touches the stream at least once per `BUMP_THRESHOLD` ledgers (roughly six
days). A touch only re-extends the record while fewer than `BUMP_THRESHOLD`
ledgers remain, so a sparser cadence — a monthly one, say — reads the stream
without postponing its expiry.

The limit comes from the contract's storage TTL policy:

- Stream records live in persistent storage with `ENTRY_TTL = 518_400`
  ledgers, roughly 30 days at five seconds per ledger.
- The contract bumps a stream record back to that target lifetime whenever a
  read or write touches it and the remaining TTL is below
  `BUMP_THRESHOLD = 103_680` ledgers, roughly six days.
- A stream that is not touched for longer than the network TTL can be archived
  by the Soroban storage system.

Beyond that range, the stream schedule is still mathematically valid, but the
record may need off-chain restoration before the contract can read or mutate it.
Clients that support long-running payroll, vesting, or subscriptions should
schedule periodic read calls or another TTL-maintenance process and should
surface restoration requirements if a record has already expired from active
storage.

## Storage Growth Over Time

The section above covers how long *one* stream's record stays alive without
being touched. This section covers the aggregate cost: how the contract's
total storage footprint grows as a deployment accumulates streams over its
lifetime, which is what anyone planning a deployment needs to size for.

### Growth is per stream created, and it never shrinks

Every successful `create_stream` call writes exactly one new persistent
`Stream(id)` entry (`contracts/stream/src/storage.rs`). Ids are assigned from
a monotonic counter and are never reused, so the number of persistent entries
the contract holds is the total count of streams ever created on that
deployment — not the number currently active. Nothing an entry point does
ever removes a `Stream(id)` entry or reclaims the space it occupies:

- There is no `delete_stream` or equivalent entry point.
- A fully withdrawn stream's record is not compacted or cleared — it keeps
  reporting its final state through every view indefinitely (see
  [README.md § Fully withdrawn stream: what each view reports](../README.md#fully-withdrawn-stream-what-each-view-reports)).
- A cancelled stream's record is likewise kept, frozen at the vested amount
  at the moment of cancellation (see
  [THREAT_MODEL.md § Invariants](../THREAT_MODEL.md#invariants)).

So storage grows monotonically with usage and has no mechanism — automatic or
manual — to shrink. A deployment that creates a million streams over its
life holds a million persistent entries forever, regardless of how many of
those streams are still active.

### A settled stream keeps its record, and keeps paying rent

"Settled" — fully vested and fully withdrawn, or cancelled — does not exempt
a stream's entry from the TTL policy described above. A settled record is
just as subject to `ENTRY_TTL` and `BUMP_THRESHOLD` as an active one: if
nothing reads or writes it again, it decays toward archival on exactly the
same schedule as a stream still being drawn down. Settling a stream stops its
*state* from changing; it does not stop the storage clock.

### Archival is per-entry, not a whole-contract event

Archival in Soroban applies to individual ledger entries, not to the contract
as a whole. A deployment with streams created at different times will have
some entries well within their TTL (because they were created or touched
recently) and others already archived (because nothing touched them for
longer than `ENTRY_TTL`), all at the same time. There is no contract-wide
expiry and no risk that activity on one stream affects another's TTL — each
`Stream(id)` entry, including `StreamCount` in instance storage, is extended
independently (`storage::get_stream`, `storage::set_stream`,
`storage::extend_instance_ttl`).

An archived entry is not deleted from the network's perspective — its data
can be restored — but it is no longer directly readable until that
restoration happens, which is an extra, explicit step (and its own fee)
before the contract can read or mutate it. A deployment with many old,
untouched, settled streams should expect a growing fraction of its entries to
be in this state over time, and should not assume that `get_stream` or any
other view will succeed against an arbitrarily old id without a restoration
step first.
