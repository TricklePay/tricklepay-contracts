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

## [Unreleased]

Nothing has been released yet and no version is tagged; `0.1.0` is still in
development. All changes below are unreleased and recorded here so downstream
consumers have a single document to track.

### Changed

- Clarified the README around cliff/no-cliff semantics, exact-end withdrawals,
  and cancellation behaviour so stream boundaries are easier to reason about.

### Added

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
