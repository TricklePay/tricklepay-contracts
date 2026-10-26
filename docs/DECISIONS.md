# Decision Records

This file records design decisions that were deliberate trade-offs, not
oversights, so a reviewer who notices the gap does not have to re-litigate it
from scratch. Each record is numbered and, once accepted, is not edited in
place — a changed decision gets a new record that supersedes it and says so.

## ADR 1: Enumeration is off-chain, not an entry point

**Status:** Accepted

### Context

There is no entry point that answers "which streams does address X have?".
`get_stream(id)` returns one stream by id; `stream_count()` returns how many
ids exist in total. Nothing returns the set of ids for a given `sender` or
`recipient`. The only on-chain way to find a party's streams today is to call
`get_stream` for every id from `0` to `stream_count() - 1` and filter
client-side, which is O(every stream ever created on the deployment) and gets
slower with every new stream, regardless of how many belong to the party
asking.

This gap is deliberate actually, not an oversight. `Created`, `Withdrawn`, and
`Cancelled` all index `sender` and/or `recipient` as event topics specifically
so an off-chain consumer can answer this query without the contract doing it
(see [README.md § Events](../README.md#events)); the indexer and web client
that consume those events live in separate repositories.

### Decision

Enumeration by party stays off-chain, answered by an indexer that subscribes
to `Created`, `Withdrawn`, and `Cancelled` and reconstructs the
address-to-stream-id mapping from their topics. The contract exposes no
`streams_of(address)` or equivalent entry point, and none is planned.

### Alternatives considered

1. **A reverse index stored on-chain** — e.g. a `Vec<u64>` of stream ids kept
   per address, appended to on every `create_stream`. Rejected: every
   creation would have to read and rewrite a collection that only grows, for
   both `sender` and `recipient`, so the cost of `create_stream` increases
   without bound as a party accumulates streams. This is exactly the
   unbounded-storage-growth cost Soroban's fee model is designed to price
   out, and it would eventually make creating or reading a heavy user's
   streams prohibitively expensive or too large to read back in one call.

2. **A paginated view entry point** — `list_streams(address, offset, limit) ->
   Vec<u64>` — backed by the same kind of on-chain index. Rejected for the
   same reason as (1): pagination only changes how the index is read, not the
   fact that the contract would still have to maintain and mutate a per-address
   list on every `create_stream`, `withdraw`, `withdraw_amount`, and `cancel`.

3. **Brute-force client iteration over `stream_count`.** Already possible
   today with no contract change, but it is O(total streams on the
   deployment) per query, paid in RPC round-trips by whoever is asking. Fine
   for a local script or a contract with a handful of streams; not something
   a production client should do for "show me my streams" at scale.

4. **Off-chain indexer on event topics (chosen).** The topics the contract
   already publishes for other reasons make `sender`/`recipient` natively
   filterable by any indexer, with no additional contract state or logic.
   This is the standard Soroban/Stellar pattern: keep the contract minimal
   and emit enough signal for indexers to reconstruct whatever view is
   needed. It also lets independent indexers build different views — by
   sender, by recipient, by token, by status — from the same event stream
   without the contract committing to any one of them.

### Consequences

- A client cannot derive "my streams" from the contract alone; it needs an
  indexer that has processed the event history (or a query to one). There is
  no on-chain fallback beyond the slow brute-force iteration in alternative 3.
- `create_stream`, `withdraw`, `withdraw_amount`, and `cancel` have storage
  and fee costs that do not grow as a party accumulates streams, because no
  per-address collection is read or mutated by any of them.
- Adding enumeration later, if it ever becomes necessary, is additive — a new
  entry point — rather than a retrofit, since no existing stream record needs
  to change shape to support it.

### What would have to change to revisit this

- A concrete case where off-chain indexing is not viable: for example, a
  deployment that must be fully self-contained with no indexer
  infrastructure, or a client environment that cannot run or trust a
  third-party indexer and must verify "my streams" from the contract alone.
- If that case materializes, the shape most consistent with this contract's
  low-footprint design is the paginated view in alternative 2, not an
  unbounded `Vec`, with the storage-growth cost accepted explicitly as a new,
  deliberate trade-off — and the resulting entry point treated as a new
  public interface surface per
  [docs/INTEGRATOR_OPERATIONS.md § Interface Stability Policy](INTEGRATOR_OPERATIONS.md#interface-stability-policy).
- Until one of those is true, this decision stands: enumeration by party is a
  client/indexer concern, not a contract concern.


## ADR: Immutable Architecture with No Admin or Initializer

- **Status**: Accepted
- **Context**: Smart contract protocols often introduce administrative keys, upgrade proxies, or initialization functions to allow pausing, parameter tuning, or code updates post-deployment. However, administrative privileges introduce critical centralization risks, single points of failure, and high-value target attack surfaces for private key compromise or malicious governance takeovers.
- **Decision**: The TricklePay smart contract architecture is strictly immutable, featuring **no admin role, no owner account, and no `initialize` function**. All contract behavior is fully deterministic and hardcoded upon deployment.
- **Consequences**:
  - **Upgrades**: The deployed contract cannot be upgraded in place. Any logic updates require deploying a brand-new contract instance and handling client-side migration.
  - **Pausing**: There is no emergency pause mechanism. Risk mitigation relies entirely on pre-deployment audit rigor and robust client-side validation.
  - **Recovery**: Administrative fund rescue or asset recovery functions do not exist; misdirected or locked funds cannot be arbitrarily withdrawn by any party.
- **Related References**: 
  - See the [Audit Readiness Threat Model](../docs/AUDIT_READINESS.md#threat-model) for central privilege risk assessments and trust assumption boundaries.

  ## ADR: Linear-Only Vesting Release Curve

- **Status**: Accepted
- **Context**: Vesting and token streaming protocols require a mathematical formula to calculate unlocked token amounts over time. While complex release curves (such as exponential, logarithmic, or S-curves) are used in some financial contexts, TricklePay's initial design requires a predictable, transparent, and computationally lightweight mechanism.
- **Decision**: Restrict the smart contract vesting logic to strictly **linear release curves** following an optional cliff period.
- **Rationale**:
  - **Simplicity & Predictability**: Linear vesting is easily understood by beneficiaries and integrators without requiring complex mathematical tooling.
  - **Gas & Compute Efficiency**: Linear calculations require minimal arithmetic operations, optimizing Soroban CPU and memory footprint.
  - **Auditability**: Straightforward formulas significantly reduce potential edge cases and rounding bugs during audits.
- **Impact of Introducing Alternative Curves**:
  If future development introduces non-linear curves (a possible future extension, not currently planned in the README), the following changes will be required:
  - **Interface / ABI**: Stream creation functions must be refactored to accept curve type enums and curve-specific parameters/coefficients.
  - **Events**: Stream initialization events must emit curve metadata so off-chain indexers know which mathematical model applies.
  - **View Functions**: Calculations in `vested_amount` and `claimable_amount` will require dynamic branching or modular curve evaluation functions rather than direct linear interpolation.
- **Related References**:
  - See the roadmap item for future vesting curve flexibility.

  ## ADR: On-Demand Derived Stream Status

- **Status**: Accepted
- **Context**: Streaming and payment contracts track state transitions across a stream's lifecycle (e.g., `Pending`, `Active`, `Completed`, or `Cancelled`). Storing explicit state values on-chain requires state-transition transactions (or cron/keeper bots) to push state changes as time advances past start/end thresholds, introducing unnecessary gas costs and transaction overhead.
- **Decision**: Stream status is **dynamically derived on every call** directly from the current ledger timestamp (`env.ledger().timestamp()`) relative to the stream's `start_time` and `end_time`. Stream status is never stored as persistent state on-chain.
- **Consequences**:
  - **Zero Transaction Overhead**: Streams transition between lifecycle states (e.g., from `Pending` to `Active` to `Completed`) seamlessly without requiring explicit state-transition transactions or maintenance calls.
  - **Storage & Gas Optimization**: Eliminates on-chain storage reads/writes for status updates, reducing overall ledger footprint and transaction gas fees.
  - **Impact on Indexers and Clients**: Off-chain indexers, UI frontends, and client SDKs cannot simply read a static status field from contract storage. Instead, they must mirror the contract's status calculation logic using the stream's temporal parameters and the latest ledger header timestamp.
- **Related References**:
  - See the status calculation logic in the [Status Module](../contracts/stream/src/status.rs).

  ## ADR: Ledger Timestamp as the Canonical Clock

- **Status**: Accepted
- **Context**: Time-dependent financial vesting and token streaming require a reliable, tamper-proof source of time within the smart contract environment. Smart contract platforms generally offer two primary mechanisms: block/ledger sequence numbers or network ledger timestamps.
- **Decision**: Use `env.ledger().timestamp()` as the canonical source of time for all vesting calculations, stream start/end thresholds, and rate evaluations.
- **Alternative Considered & Rejected**: 
  - *Ledger Sequence Numbers*: Using discrete ledger count increments as a proxy for time passing.
  - *Why it was rejected*: Ledger sequences do not map intuitively to real-world calendar time or human expectations (e.g., scheduling a stream to unlock on a specific date versus calculating exact ledger heights). Furthermore, ledger close times on Stellar are highly stable (~5 seconds per ledger), making wall-clock timestamps superior for predictability. While ledger timestamps are bounded by ledger close resolution rather than continuous sub-second precision, they provide an ideal balance of auditability and real-world utility.
- **Consequences**:
  - **Intuitive Scheduling**: Vesting cliffs and durations align directly with Unix epoch timestamps, making client integration and user comprehension straightforward.
  - **Granularity Limits**: Time calculations are precise to the ledger close interval (~5 seconds) rather than wall-clock milliseconds.
- **Related References**:
  - See the detailed discussion on clock precision and assumptions in the [Ledger time as the source of truth section of the README](../README.md#ledger-time-as-the-source-of-truth).


  ## ADR: `i128` Amount Type and `i64::MAX` Stream Amount Ceiling

- **Status**: Accepted
- **Context**: Soroban token interfaces utilize `i128` data types for token balances and transfers to support high-precision assets. However, calculating linear vesting streams requires multiplying total amounts by elapsed time and duration intervals, which creates potential integer overflow vectors during arithmetic operations.
- **Decision**: Token amounts are stored and processed as `i128` for complete compatibility with the Stellar / Soroban token standard, but `create_stream` strictly enforces an upper limit capped at `i64::MAX` for individual stream allocations.
- **Consequences**:
  - **Arithmetic Safety**: Capping individual streams at `i64::MAX` guarantees that intermediate multiplication steps during vesting calculations cannot overflow standard integer boundaries.
  - **Largest Streamable Amount**: The maximum amount streamable in a single contract instance is `i64::MAX` (approximately $9.22 \times 10^{18}$ base units).
  - **Exclusions**: Assets with astronomical token supplies or extreme decimal precision exceeding `i64::MAX` cannot be streamed in a single stream instance and must be split across multiple streams if necessary.
- **Related References**:
  - See the detailed discussion in the [Amount Ceiling section of the README](../README.md#amount-ceiling).

  