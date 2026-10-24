# Contributing to TricklePay Contracts

Thank you for your interest in improving the TricklePay contracts. This short
guide covers the local setup, the checks that must pass, and how to open a pull
request for this repository.

TricklePay's broader contribution conventions — coding standards, the commit and
review process, governance, and where each piece of the project lives — are in the
[shared contribution guide](https://github.com/TricklePay/docs/blob/main/CONTRIBUTING.md).
Please read it before you start. This file only adds what is specific to this
repository and should be kept short rather than duplicating the shared guide.

> **Security**: this repository holds fund-moving code. Do **not** open a public
> issue for a security vulnerability — follow the responsible disclosure process in
> [SECURITY.md](SECURITY.md) instead.

## Setup

Prerequisites:

- **Rust stable** with the `wasm32v1-none` target. `rust-toolchain.toml`
  selects the moving `stable` channel, not an exact compiler version; install
  the toolchain and target with [rustup](https://rustup.rs). The reproducible
  WASM build instructions were verified with Rust 1.96.0, so record and use the
  same compiler version when reproducing a deployment build.
- **The [Stellar CLI](https://developers.stellar.org/docs/tools/cli)**, only
  needed if you deploy the contract to a network.

Clone and build:

```bash
git clone https://github.com/TricklePay/tricklepay-contracts.git
cd tricklepay-contracts
cargo test
```

## Required checks

All of the following must pass before opening a pull request:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo audit --deny warnings
bash ./scripts/check-release-overflow.sh
./scripts/diff-interface.sh
```

> **Tip:** You can use the project's cargo aliases (defined in `.cargo/config.toml`) for shorter commands: `cargo fmt-check`, `cargo lint`, and `cargo test`. These run the exact same checks as the `Makefile` targets.

CI runs the same checks on every push and pull request. The audit command uses the
allowlist in `.cargo/audit.toml`.

The vesting and status property tests use Proptest's default 256 cases per
property. To run more cases before a release or after changing property-tested
logic, set `PROPTEST_CASES` for the test command:

```bash
PROPTEST_CASES=10000 cargo test
```

In PowerShell, use:

```powershell
$env:PROPTEST_CASES = "10000"; cargo test
```

When Proptest finds a failure, it saves the seed under the relevant source
directory's `proptest-regressions/` folder (for example,
`contracts/stream/src/proptest-regressions/` for the vesting and status
properties). Proptest replays saved failures automatically when that property
runs again. To rerun just one property, pass its test name to Cargo, for
example `cargo test vested_between_zero_and_total`.

If you intentionally changed the contract interface, update the snapshot before opening your PR:
```bash
make wasm
stellar contract inspect --wasm target/wasm32v1-none/release/tricklepay_stream.wasm > docs/interface.txt
```
The equivalent target is `make snapshot`. Run `make check-markdown-links` when
changing documentation, and use `make verify-deployment CONTRACT_ID=C... NETWORK=testnet`
to compare a deployment with the local build.

## Contract documentation in the deployed spec

Rustdoc comments on `#[contractimpl]` items, `#[contracttype]` types, and
`#[contracterror]` errors are included in the deployed WASM spec. They increase
the binary size and deployment fee, so keep them concise. Use `make size` to
check the binary size and `make snapshot` to inspect and update the interface
snapshot.

## Testing

**Tests must control the ledger clock explicitly.** A test may never depend on
wall-clock time: do not read the current time, do not sleep to let time pass,
and do not assert against a duration measured in real seconds.

`Env::default()` starts the test ledger at a fixed timestamp, and the suite only
moves time forward by assigning it. Time in a test is therefore a value you
choose, not a value that drifts.

The contracts have two clocks, and `StreamTest` in
[`contracts/stream/src/tests/helpers.rs`](contracts/stream/src/tests/helpers.rs) exposes a helper
for each:

| Helper | Underlying call | Use it for |
| ------ | --------------- | ---------- |
| `set_time(ts)` | `env.ledger().set_timestamp(ts)` | the stream schedule: start, cliff, withdrawal eligibility |
| `set_sequence(seq)` | `env.ledger().set_sequence_number(seq)` | entry lifetimes, which are counted in ledgers rather than seconds |

`withdraw_releases_vested_in_steps` is the reference example. It creates a
stream, then steps the clock to the midpoint, the three-quarter point, and the
end, asserting the exact amount released at each step:

```rust
let t = StreamTest::setup(1_000);
t.set_time(100);
let id = t.contract.create_stream(
    &t.sender,
    &t.recipient,
    &t.token_address,
    &1_000,
    &100,
    &1_100,
    &100,
);

// Midpoint: half has vested.
t.set_time(600);
assert_eq!(t.contract.withdraw(&id), 500);
assert_eq!(t.token.balance(&t.recipient), 500);
```

When you need the raw API, `env.ledger().set_timestamp(...)` is equivalent; a
few tests in the same file call it directly.

### Why time-dependent tests are rejected

A test that reads the host clock has no stable answer, so the suite stops being
a signal:

- **The result depends on when it runs.** The same code passes and fails
  depending on machine speed, load, and clock resolution. A boundary case such
  as "is the current time still before the cliff?" then resolves differently on
  a busy CI runner, and the failure surfaces far from its cause.
- **The interesting cases are unreachable.** Stream behaviour is defined across
  a timeline: before the start, at a cliff, mid-stream, exactly at the end, and
  after it. You cannot wait for those moments, and pinning a stream to the host
  clock only ever exercises whichever one the test happens to run at. Assigning
  the timestamp makes each case a single line.
- **The suite gets slower.** A test that sleeps pays that sleep on every run,
  locally and in CI, to reach a timestamp you could have assigned outright.

This also keeps the tests honest about the contracts. Contract code reads the
current time from the ledger (`env.ledger().timestamp()`, for example in
`create_stream` and `withdraw`) and never from the host. A test that injects a
host timestamp would break the assumption the rest of the suite relies on, and
could let a regression that reintroduced wall-clock time into the contract pass
unnoticed.

### Writing a test for a new behaviour

All tests live in [`contracts/stream/src/tests/`](contracts/stream/src/tests/)
 — the suite is split into one module per area (see `tests/mod.rs`), so the fastest way to learn its conventions is to read
the tests around the behaviour you are touching. `StreamTest::setup` builds the
whole environment for you: a registered stream contract, a Stellar asset
token, a sender funded with the balance you pass, and `mock_all_auths()` so
calls do not need signatures.

A new test is usually four steps:

1. **Build the fixture.** `StreamTest::setup(sender_balance)` returns `t`, a
   struct with the contract client (`t.contract`), the token client (`t.token`),
   the participant addresses (`t.sender`, `t.recipient`), and the clock helpers.
2. **Pin the clock, then create the stream.** Call `t.set_time(...)` before
   `create_stream`: the contract rejects a window whose end is already in the
   past, so the schedule only exists once the clock is where you want it. Most
   tests use the window `start = 100`, `end = 1_100` with `cliff = 100`, which
   is the no-cliff case (`cliff_time == start_time`). Pass explicit timestamps
   as plain numbers rather than computing them from the current time.
3. **Act and assert on the schedule.** Advance the clock with `t.set_time(...)`
   to each point of interest — before the start, at a cliff, the midpoint, the
   exact end, past the end — and assert the exact amount at each step with
   `assert_eq!`. Put a comment above each step naming the moment, as
   `withdraw_releases_vested_in_steps` (the reference example above) does.
4. **Assert rejections change nothing.** For a call that must fail, use the
   generated `try_` client method and expect the typed error, then assert the
   balances and stored state are untouched:

   ```rust
   let balance_before = t.token.balance(&t.recipient);
   assert_eq!(
       t.contract.try_withdraw_amount(&id, &400),
       Err(Ok(StreamError::InsufficientBalance))
   );
   assert_eq!(t.token.balance(&t.recipient), balance_before);
   ```

   The double `Err(Ok(...))` shape is how the SDK client reports a contract
   error from a `try_` call: outer layer for the invocation, inner for the
   decoded `StreamError`. Never assert on panics for errors the contract
   returns as values.

Putting it together — a test that the cliff withholds everything until
`cliff_time` and then releases the accrued amount at once:

```rust
/// Withholding before the cliff, then the one-step release at the cliff.
#[test]
fn nothing_is_withdrawable_before_the_cliff() {
    // A fresh contract, token, and funded sender; auth is mocked.
    let t = StreamTest::setup(1_000);

    // Pin the clock before creating the stream: the schedule's end must
    // be in the future at creation.
    t.set_time(100);

    // The standard window [100, 1100] with the cliff at 600. Cliff equal
    // to start (100) would be the no-cliff case.
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &600,
    );

    // Past the start, before the cliff: the gate withholds everything.
    t.set_time(300);
    assert_eq!(t.contract.withdrawable(&id), 0);

    // At the cliff the accrued amount unlocks at once and vesting
    // continues linearly from there.
    t.set_time(600);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
}
```

Conventions the suite expects:

- **Name the behaviour, not the entry point.** `withdraw_at_exact_end`, not
  `test_withdraw_2`. A reader should know what passing means from the name.
- **Give every test a doc comment** stating the property it pins, like the
  tests around the storage TTL do.
- **Timestamps are values you choose.** Write `100` and `1_100` directly;
  never derive a timestamp from another reading of the clock.
- **Assert on state, not just return values.** Pair an amount with the token
  balances (`t.token.balance(&t.recipient)`, `t.token.balance(&t.contract.address)`)
  so a transfer bug cannot hide behind a correct number.
- **Prefer a fixture helper before reaching for the raw ledger API.**
  `StreamTest` also exposes `set_sequence` for ledger lifetimes, and storage
  introspection such as `stream_ttl`, `persistent_has`, and `set_stream_count`
  for boundary cases no entry point can reach; read the fixture in
  `tests/helpers.rs` before duplicating one of them.

When a property test finds a failing input, force-add its generated file from
`proptest-regressions/` together with the fix for that failure, using
`git add -f <path-to-regression-file>`. These files are ignored by default, so
force-adding is required to share the regression seed with every contributor.

For events, use the fixture helpers instead of decoding XDR by hand:
`event_publishers()` returns who published the latest invocation's events in
order (the token contract's transfer, then the stream contract's own event),
and `assert_latest_stream_event_topics` compares the latest event's topics and
data against an expected `ContractEvent`.

When you are done, run the required checks above — `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`, or
`make check` for all three — before opening the pull request.

## Commit messages

Start every commit subject with a type prefix, a colon, and a short summary in
the imperative mood, lowercase, with no trailing period:

```text
<type>: <summary>
```

| Type       | Use for                                                        |
| ---------- | -------------------------------------------------------------- |
| `feat`     | a new contract behaviour or entry point                        |
| `fix`      | a bug fix in contract code                                     |
| `test`     | adding or changing tests only                                  |
| `docs`     | README, guides, and doc comments                               |
| `refactor` | a code change that does not alter behaviour                    |
| `style`    | formatting only (`cargo fmt`)                                  |
| `build`    | toolchain, `Cargo.toml`, `Makefile`, or build scripts          |
| `ci`       | CI workflow configuration                                      |
| `chore`    | maintenance that fits none of the above                        |

For example:

```text
fix: reject cancel on a completed stream
test: add withdraw_amount boundary tests
docs: document the storage lifetime constants
```

Issues often suggest a commit message. Use it as written when it fits the
change. Reference the issue in the commit body or the pull request description
(`Closes #123`), not in the subject.

## How to open a pull request

1. Create a branch from `main`, named after the issue you are working on
   (for example `chore/issue-165`).
2. Make a focused change and run the checks above.
3. If your change modifies contract ABI, user-facing behavior, or fixes a bug, update `CHANGELOG.md` per the guidelines below.
4. Push the branch to your fork and open a pull request against `main`.
5. Describe the change, the motivation, and how you verified it, and link the
   issue you are addressing (for example `Closes #123`). If your pull request modifies contract logic, ensure it meets the [Contract change checklist](#contract-change-checklist).
6. Be responsive to review feedback; follow-up commits during review are fine.

## Contract change checklist

When your pull request changes contract logic or interface, please verify:

- [ ] **Tests:** New or changed behavior is covered by tests that control the ledger clock explicitly.
- [ ] **Changelog:** The change is documented in `CHANGELOG.md` following the guidelines below.
- [ ] **Interface:** If the contract interface changed, the snapshot in `docs/interface.txt` has been updated and `make check` passes.

## Updating the changelog

Contributions must update [`CHANGELOG.md`](CHANGELOG.md) under the `## [Unreleased]` section whenever a pull request introduces:

- **Public ABI Changes**: New or modified contract entry points, changes to `StreamError` codes or variants, or additions/modifications to event payloads (prefix entry with `**ABI:**`).
- **User-facing Behavior Changes**: Modifications to validation rules, timing or schedule semantics, refund behaviors, or contract parameter caps.
- **Bug Fixes or Breaking Changes**: Fixes to contract logic or state handling, or removal/retirement of existing behaviors or error codes.

### Changelog Entry Format

Entries in [`CHANGELOG.md`](CHANGELOG.md) must follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) standards under `## [Unreleased]` using one of the existing subheadings (`### Added`, `### Changed`, `### Deprecated`, `### Removed`, `### Fixed (non-ABI)`).

Format ABI entries with the bold `**ABI:**` tag, function or error signature, description, and date:

```markdown
- **ABI:** `create_stream(sender, recipient, token, total_amount, start_time, end_time, cliff_time) → u64` — description of function. Added YYYY-MM-DD.
- **ABI:** `StreamError::InvalidParticipant`, error code `13`. Description of validation error. Added YYYY-MM-DD.
```

Format non-ABI behavior changes or bug fixes concisely:

```markdown
- Clarified README around cliff/no-cliff semantics so stream boundaries are easier to reason about.
- `create_stream` now validates that `total_amount` does not exceed `i64::MAX`. Fixed YYYY-MM-DD.
```

## Release process

Releases are cut by the maintainers (`@TricklePay/maintainers`, see
[CODEOWNERS](.github/CODEOWNERS)), not by the author of an individual pull
request. A release happens when the maintainers decide the accumulated
`## [Unreleased]` changes in `CHANGELOG.md` are ready to ship — there is no
fixed schedule.

### Cutting and tagging a release

1. Confirm `main` is green: `make check` (formatting, lint, tests, audit)
   plus `./scripts/diff-interface.sh`, so the committed `docs/interface.txt`
   matches a fresh `make wasm` build.
2. Pick the version number under
   [Semantic Versioning](https://semver.org/spec/v2.0.0.html): a breaking
   interface change (see
   [docs/INTEGRATOR_OPERATIONS.md § Interface Stability Policy](docs/INTEGRATOR_OPERATIONS.md#interface-stability-policy))
   is a major bump once the project is at `1.0.0` or later; before `1.0.0`,
   follow the usual pre-1.0 convention of treating a breaking change as a
   minor bump and everything else as a patch.
3. In a release commit on `main`:
   - Rename `## [Unreleased]` in `CHANGELOG.md` to `## [X.Y.Z] - YYYY-MM-DD`,
     and open a new, empty `## [Unreleased]` section above it for whatever
     comes next.
   - Bump `version` in the workspace `Cargo.toml` to match.
   - Run `make wasm-hash` and record its `sha256:` output in the tag
     annotation — this is the hash anyone can later check a live deployment
     against (see
     [Verifying a deployment](README.md#verifying-a-deployment)).
4. Tag that commit with an annotated tag, `vX.Y.Z`
   (`git tag -a vX.Y.Z -m "..."`), and push it (`git push origin vX.Y.Z`).
   The tag is what makes a specific `CHANGELOG.md` entry and a specific,
   reproducible WASM build addressable by name.

### How a tag relates to a deployed contract

A tag names a point in source history, **not** a deployed contract address —
the two are linked only through the reproducible build:

- The tagged commit, built with the same exact compiler version and target
  (`wasm32v1-none`) used for deployment, produces the corresponding WASM binary
  and SHA-256 hash. The build instructions in this repository were verified
  with Rust 1.96.0; `rust-toolchain.toml` selects the moving `stable` channel,
  so check the compiler version used for the deployment rather than assuming
  the current stable compiler will match (see
  [Verifying a deployment](README.md#verifying-a-deployment)).
- Deploying that WASM (`make deploy IDENTITY=...` / `scripts/deploy.sh`)
  creates a contract at a new address — the `C...` address the deploy script
  prints. Because the contract has no upgrade path (see
  [docs/INTEGRATOR_OPERATIONS.md § Redeployment Without Upgradeability](docs/INTEGRATOR_OPERATIONS.md#redeployment-without-upgradeability)),
  that address and the tag it was built from stay fixed to each other for
  the life of that deployment.
- The same tag can be deployed more than once — once per network (testnet,
  mainnet, a future chain), or again on the same network if a redeployment
  is needed — and each deployment gets its own address. There is
  deliberately no registry inside the contract mapping tags to addresses;
  record which tag, network, and address go together wherever deployments
  are announced, following the migration flow in
  [docs/INTEGRATOR_OPERATIONS.md § Redeployment Without Upgradeability](docs/INTEGRATOR_OPERATIONS.md#redeployment-without-upgradeability).
- To check whether a live contract matches a given tag: fetch its bytecode
  (`stellar contract fetch`), hash it, check out the tag, rebuild, and
  compare hashes — the same procedure as
  [Verifying a deployment](README.md#verifying-a-deployment), just starting
  from a tag instead of a working-tree checkout.

### Who performs a release

Only the maintainers cut and push a release tag. This is explicitly outside
the standard contributor pull-request flow in
["How to open a pull request"](#how-to-open-a-pull-request) above, which ends
at merging to `main`. A contributor's responsibility stops at an accurate
`## [Unreleased]` entry in `CHANGELOG.md`; deciding when those changes ship,
tagging them, and deploying from that tag is a maintainer action.

## Code of conduct

Be respectful and constructive in all project spaces. See the
[shared contribution guide](https://github.com/TricklePay/docs/blob/main/CONTRIBUTING.md)
for the full expectations.
