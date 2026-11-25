# Changelog

Notable changes to the `stream` contract. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Error codes are part of the public ABI: off-chain consumers match on the
integer, not the variant name. Every change to the set of codes is recorded
here and marked **ABI**, whether or not any reachable behaviour changes with
it.

Event payloads are part of the public ABI: indexers decode fields by name and
position. Every addition or removal of a field is marked **ABI**.

Entry points (callable functions) are part of the public ABI. Every addition
is marked **ABI**.

## How to read this changelog

Entries are grouped under the standard Keep a Changelog headings — **Added**,
**Changed**, **Removed**, **Fixed** — which describe the *nature* of a change,
not whether it affects you. To find out which entries can affect you, look at
the inline **ABI** tag on each entry, not at the section heading it sits
under:

- **Added** and **Removed** entries below are always interface changes:
  introducing or retiring an error code, event field, or entry point is
  inherently a public-ABI change, so every entry under those two headings
  carries the **ABI** tag.
- **Changed** and **Fixed** entries are mixed. Some change the interface (an
  event payload gaining a field, a previously-unreachable error code becoming
  reachable); others are internal only (a build-profile tweak, a README
  clarification, a bug fix that doesn't change what the contract returns).
  Judge each one by its own **ABI** tag rather than the heading — a `Fixed
  (non-ABI)` section, for example, can still contain individual entries tagged
  **ABI** when the fix is what makes a new error code reachable.

If you maintain a generated client, SDK, or indexer that matches on error
codes, decodes event topics or fields, or calls an entry point directly, the
**ABI**-tagged entries are the ones that can require a change on your side.
Entries without the tag are safe to skip for that purpose.

## [Unreleased]

Nothing has been released yet and no version is tagged; `0.1.0` is still in
development. All changes below are unreleased and recorded here so downstream
consumers have a single document to track. See
[CONTRIBUTING.md § Release process](CONTRIBUTING.md#release-process) for how
this section turns into a tagged release and how a tag relates to a deployed
contract.

### Added

- **ABI:** Added `transfer_recipient(id, new_recipient) -> ()`, allowing the
  current recipient to assign withdrawal rights to another address.
- **ABI:** Added `RecipientTransferred` event with indexed `old_recipient` and
  `new_recipient` topics and `stream_id` in the data payload.
- **non-ABI:** Accepted `docs/DESIGN_RECIPIENT_TRANSFER.md`, recording transfer
  authorization, validation, escrow, cancellation, and indexer decisions.
- **ABI:** Added `created_at: u64` to the `Stream` struct and the `Created` event
  payload, recording the ledger timestamp at which the stream was created.
  **Migration note:** regenerate generated clients and update event decoders;
  `created_at` is the last `Created` data field, so consumers that decode the
  payload positionally must read it after `cliff_time`. Stream records written
  by a pre-`created_at` contract do not contain the field and cannot be read as
  the new `Stream` type; backfill them from the creation ledger close time or
  migrate them before upgrading a deployment.
- **ABI:** Added `rate_per_second(id) -> i128`, rounding down by stream duration and returning `0` for cancelled streams.
- **ABI:** Added `withdrawable_many(ids) -> Vec<i128>`, returning zero for unknown ids and enforcing `MAX_BATCH_SIZE`.
- **ABI:** Added `withdraw_many(ids) -> Vec<i128>` with per-stream transfers and events, one authorization per distinct recipient, and `NothingToWithdraw` when the batch pays nothing.
- **ABI:** Added `withdraw_to(id, destination, amount) -> i128` entry point allowing the stream recipient to withdraw vested tokens directly to a designated address.
- **ABI:** Extended `Withdrawn` event payload with `destination: Address` recording the address receiving the transferred tokens.
- **non-ABI:** Accepted `docs/DESIGN_WITHDRAW_TO.md` specifying `withdraw_to` semantics, authorization, and event format.
- **ABI:** Added `topup_stream` entry point allowing the stream sender to deposit additional tokens, increasing `total_amount` while preserving duration and schedule (Option B: increase rate).
- **ABI:** Emitted `ToppedUp` event (`sender` topic, `stream_id`, `added_amount`, `new_total_amount`, `new_end_time`) on successful stream top-up.
- **non-ABI:** Accepted `docs/DESIGN_TOPUP.md` with resolved design decisions on top-up semantics and error handling.

### Changed

- **non-ABI:** Added JavaScript SDK invocation, view query, and event decoding examples to the README.
- **non-ABI:** Added a Mermaid state transition diagram for `StreamStatus` to the README, distinguishing time-driven vs transaction-driven triggers.
- **non-ABI:** Documented the storage TTL extension split between individual stream views and read-only views (`get_streams`, `cancel_preview`) across storage docs, the threat model, and the README, with a test pinning `get_streams` against TTL decay.
- **non-ABI:** Standardized the header in `DESIGN_BATCH_STREAM_CREATION.md` and added design document status definitions and a template to `docs/README.md`.
- **non-ABI:** Embedded contract metadata keys `version` and `repository` are
  now shown by `stellar contract inspect`.
- Clarified the README around cliff/no-cliff semantics, exact-end withdrawals,
  and cancellation behaviour so stream boundaries are easier to reason about.
- Removed the one-line private helper `vested_amount_at` in `contract.rs`; call sites
  in `vested` and `vested_at` now invoke `stream.vested_amount` directly and
  consistently with other calculation sites.
- Centralized `Cancelled` event publishing into `events::publish_cancelled` in
  `events.rs`, aligning cancellation publishing with `publish_created` and `publish_withdrawn`.
- Documented JSON-RPC `getEvents` filter examples for `Created` (by sender and by recipient),
  `Withdrawn` (by recipient), and `Cancelled` (by sender) in `README.md`, including
  topic XDR base64 symbols, ledger retention windows, and pagination cursor semantics.
- Documented instruction, memory, and ledger entry resource costs per entry point
  in `README.md` measured against `soroban-sdk = "=25.3.2"`, with regression test ceilings
  enforced in `tests/budget.rs`.

### Added

- Resource budget regression test suite (`contracts/stream/src/tests/budget.rs`)
  measuring and asserting documented upper bounds with headroom on CPU instructions,
  memory, and storage ledger reads/writes for every contract entry point.
- Boundary and duplicate stream ID test coverage for batch queries (`get_streams`),
  verifying that exactly `MAX_BATCH_SIZE` (20) IDs succeed, duplicates preserve
  request order, and unknown IDs return `None` without failing mixed batches.
- Fee-on-transfer token test coverage pinning the failure modes of short deposits
  (final withdrawal failure and cross-stream collateral draining), cross-referencing
  the threat model decision on balance-difference checks.

- **ABI:** `create_stream(sender, recipient, token, total_amount, start_time,
end_time, cliff_time) → u64` — opens a new stream, pulls the full
  `total_amount` from the sender into the contract, and returns the assigned
  stream id. Added 2026-06-17.

- **ABI:** `withdraw(id) → i128` — lets the recipient pull all vested-but-not-yet-
  withdrawn tokens in one call. Returns the amount transferred. Added
  2026-06-17.

- **ABI:** `cancel(id) → i128` — lets the sender stop a stream early. The vested
  portion stays claimable by the recipient; the unvested remainder is refunded
  to the sender. Returns the refund amount. Added 2026-06-17.

- **ABI:** `get_stream(id) → Stream` — returns the full stream record for the
  given id. Added 2026-06-18.

- **ABI:** `withdrawable(id) → i128` — returns the amount the recipient can
  withdraw right now (vested minus already withdrawn). Added 2026-06-18.

- **ABI:** `vested(id) → i128` — returns the total amount vested so far,
  including anything already withdrawn. Added 2026-06-18.

- **ABI:** `status(id) → StreamStatus` — returns the lifecycle state of a stream
  (`Pending`, `Streaming`, `Completed`, or `Cancelled`) at the current ledger
  time. Added 2026-06-18.

- **ABI:** `stream_count() → u64` — returns the number of streams created so
  far; valid ids run from `0` to `stream_count - 1`. Added 2026-06-18.

- **ABI:** `withdraw_amount(id, amount) → i128` — partial withdrawal; lets the
  recipient take a specific amount up to the currently withdrawable balance
  rather than the full available sum. Fails with `InsufficientBalance` (code 8)
  if the requested amount exceeds what is available. Returns the amount
  transferred. Added 2026-06-21.

- **ABI:** `locked(id) → i128` — returns the portion of `total_amount` that has
  not yet vested (i.e. still locked in the contract and not yet claimable by
  the recipient). A cancelled stream always returns `0` because cancellation
  freezes the total at the vested amount. Added 2026-06-21.

- **ABI:** `progress(id) → u32` — returns vesting progress in basis points, from
  `0` (nothing vested) to `10000` (fully vested). Useful for progress
  indicators without fetching the full stream record. Added 2026-07-11.

- **ABI:** `stream_exists(id) → bool` — checks whether a stream record exists,
  including cancelled and completed streams, without writing state or
  publishing events. Added 2026-10-20.

- **ABI:** `vested_at(id, timestamp) → i128` — projects the vested amount at a
  specified Unix timestamp using the contract's vesting calculation. Added
  2026-10-20.

- **ABI:** `max_amount() → i128` — returns the maximum amount accepted by
  `create_stream`, so clients can validate against the deployed contract.
  Added 2026-10-20.

- **ABI:** `cancel_preview(id) → Settlement` — returns the settlement amounts
  (`refund` to sender and `recipient_remaining` to recipient) that a `cancel` call
  at the current ledger time would produce, without modifying contract state or
  publishing events. Added 2026-10-25.

- **ABI:** `get_streams(ids) → Vec<Option<Stream>>` — reads up to `MAX_BATCH_SIZE`
  (20) stream records in request order, returning `None` for unknown or expired
  ids, without modifying storage or publishing events. Added 2026-10-25.

- **ABI:** `MAX_BATCH_SIZE: u32 = 20` — constant defining the maximum number of
  stream ids that `get_streams` accepts in a single call. Added 2026-10-25.

### Changed

- **ABI:** `Created` event payload extended with three schedule fields:
  `start_time: u64`, `end_time: u64`, and `cliff_time: u64`. Previously the
  data portion carried only `(id, token, total_amount)`; it now carries
  `(id, token, total_amount, start_time, end_time, cliff_time)`. Indexers that
  were recording only the first three data fields must update their decoders to
  handle the additional fields. Changed 2026-07-11.

- The `[profile.release]` build profile is tuned for WASM artifact size:
  `opt-level = "z"`, `lto = true`, `codegen-units = 1`, and `panic = "abort"`
  reduce the compiled size, while `strip = "symbols"` and `debug = 0` drop
  metadata not needed in a deployed artifact. `overflow-checks` stays `true`
  even in release, trading a small amount of size for safety against silent
  arithmetic overflow in token amounts. This is a build-tooling change with no
  effect on contract behaviour or the ABI.

### Added

- **ABI:** `StreamError::StreamCountExhausted`, error code `12`. Stream ids come
  from a monotonic `u64` counter, and `create_stream` previously incremented it
  unchecked. At `u64::MAX` that increment would wrap to zero and the next
  stream would be written over the record already holding id `0`, destroying it
  along with the claim on its locked tokens. Creation now fails with this code
  instead, checked before any tokens move so a rejected call costs the caller
  nothing.

  Reaching the bound takes `u64::MAX` successful creations, so no existing
  caller can observe this in practice; it is a fail-closed guard, not a new
  routine failure mode.

- **ABI:** `StreamError::InvalidParticipant`, error code `13`. `create_stream`
  now rejects this contract's own address as `sender`, `recipient`, or `token`,
  checked before any tokens move. Each role previously failed in its own
  unhelpful way:
  - As `recipient` the call **succeeded**, locking the tokens permanently.
    `withdraw` requires the recipient's authorization and the contract cannot
    sign for itself, so nothing could ever claim them.
  - As `sender` the transfer failed inside the token contract, which returned
    its own `BalanceError`. That code is `10`, the same number as
    `AmountTooLarge`, so the generated client decoded a token-contract failure
    as an unrelated stream error.
  - As `token` the call aborted at the host level with no typed error, because
    this contract exposes no `transfer` entry point.

- **ABI:** `StreamError::BatchTooLarge`, error code `14`. Returned by `get_streams`
  when more than `MAX_BATCH_SIZE` (20) stream ids are requested in a single call.

### Changed

- `create_stream` now rejects a stream whose `sender` and `recipient` are the
  same address, with `InvalidParticipant` (code `13`). Such a stream only
  locked the caller's own tokens and returned them over time; it was accepted
  before. **This rejects input that previously succeeded** — callers using a
  self-stream as a deliberate self-lockup need to change approach.

- `create_stream`'s validation order is now part of its documented contract:
  authorization, participants, amount, schedule, capacity, and only then
  effects. All validation runs before any token transfer or storage write, so
  a rejected call leaves nothing behind, and an argument list that breaks
  several rules always reports the earliest group rather than whichever check
  happens to run first. No valid call changes behaviour.

### Removed

- **ABI:** `StreamError::Unauthorized`, error code `2`, is removed. Nothing in
  the contract ever constructed it. Authorization is enforced with
  `require_auth()`, which panics with a host auth error before the entry point
  body runs, so a caller could never have received code `2` — it advertised a
  failure mode that did not exist. No reachable behaviour changes; clients
  matching on it can drop that branch.

  The remaining codes keep their original values — `StreamNotFound` is still
  `1`, `InvalidTimeRange` still `3`, through `InsufficientBalance` at `8` — so
  existing indexers continue to decode every error they can actually observe.
  Code `2` is retired and will not be reassigned; the gap in the numbering is
  deliberate. Removed 2026-08-08.

### Fixed (non-ABI)

- Instance TTL is now extended on every `create_stream` call. Previously the
  `StreamCount` storage entry could expire on a long-idle contract, making it
  impossible to create new streams. Existing streams and withdrawals were not
  affected. Fixed 2026-08-08.

- **ABI:** `cancel` now returns `StreamAlreadyCompleted` (code `9`) when called
  on a stream whose `end_time` has already passed. Previously the call would
  succeed and issue a zero-refund, leaving the stream in an inconsistent
  cancelled state after full vesting. Callers that relied on cancelling
  completed streams must handle the new error code. Fixed 2026-08-11.

- **ABI:** `create_stream` now validates that `total_amount` does not exceed
  `i64::MAX` (≈ 9.2 × 10¹⁸ stroops). Amounts above this cap are rejected with
  `AmountTooLarge` (code `10`). The bound prevents `i128` overflow in the
  vesting arithmetic, where `total_amount` is multiplied by an elapsed-time
  value that can be as large as `u64::MAX`. Fixed 2026-08-11.

- **ABI:** `create_stream` now rejects a time window that ends entirely in the
  past. If `end_time` is at or before the current ledger timestamp the call
  fails with `StreamWindowInPast` (code `11`). A stream whose end time has
  already passed would be 100 % vested on creation — effectively an immediate
  transfer. Use a token transfer directly instead. Fixed 2026-08-20.

---

### Error code reference

| Code | Variant                  | Status  |
| ---- | ------------------------ | ------- |
| 1    | `StreamNotFound`         | Active  |
| 2    | _(retired)_              | Retired |
| 3    | `InvalidTimeRange`       | Active  |
| 4    | `InvalidAmount`          | Active  |
| 5    | `InvalidCliff`           | Active  |
| 6    | `AlreadyCancelled`       | Active  |
| 7    | `NothingToWithdraw`      | Active  |
| 8    | `InsufficientBalance`    | Active  |
| 9    | `StreamAlreadyCompleted` | Active  |
| 10   | `AmountTooLarge`         | Active  |
| 11   | `StreamWindowInPast`     | Active  |
| 12   | `StreamCountExhausted`   | Active  |
| 13   | `InvalidParticipant`     | Active  |
| 14   | `BatchTooLarge`          | Active  |
