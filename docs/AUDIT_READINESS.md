# Audit Readiness Checklist

This repository treats a formal third-party audit as the gate between the
`stream` contract and production use (see the "Security model summary" in
[SECURITY.md](../SECURITY.md) and the risk this contract carries without one).
This checklist is what "ready to send to an auditor" means in concrete terms:
the invariants are written down, the threat model is current, and the test
suite exercises both.

Work through it before engaging an auditor, and re-check any item touched by
a change since the last audit. An item that cannot be checked off is a gap to
close first, not something to explain away in the engagement kickoff call.

## Invariants

- [ ] **Every contract invariant is written down and still accurate.**
      Lives in [THREAT_MODEL.md § Invariants](../THREAT_MODEL.md#invariants) —
      escrow bounds, `withdrawn` monotonicity, cancellation freezing, the
      sender/recipient authorization split, id uniqueness, the `total_amount`
      bound, and the cliff/start/end ordering. Each entry names the file and
      function that enforces it; confirm those references still point at the
      right code after any change to `contract.rs`, `vesting.rs`, or
      `storage.rs`.
- [ ] **Every invariant has a corresponding test.** Cross-check each bullet in
      the Invariants section against
      [`contracts/stream/src/tests/`](../contracts/stream/src/tests/mod.rs):
      `withdrawal.rs` and `cancellation.rs` for the accounting invariants,
      `auth.rs` for the authorization split, `creation.rs` for id issuance and
      the `total_amount`/cliff bounds. An invariant with no test exercising it
      is a gap, not a documentation-only fix.
- [ ] **The vesting arithmetic has property-based coverage, not just example
      cases.** Lives in the `proptest` block at the bottom of
      [`contracts/stream/src/vesting.rs`](../contracts/stream/src/vesting.rs)
      (`withdrawable_between_zero_and_vested`,
      `withdrawable_equals_vested_minus_withdrawn_when_withdrawn_le_vested`,
      and the sibling properties above them). These are what an auditor will
      look for before trusting the rounding and clamping behavior described
      in [README.md § Integer rounding](../README.md#integer-rounding) and
      [§ Amount ceiling](../README.md#amount-ceiling).

## Threat model

- [ ] **[THREAT_MODEL.md](../THREAT_MODEL.md) reflects the deployed design.**
      In particular: the "No pause mechanism" section, the authorization
      model, and the "Trust assumptions about the token contract" section.
      If a change adds any privileged entry point, admin key, or pause path,
      this document is now wrong and must be updated before an audit, not
      after.
- [ ] **Out-of-scope risks are still the right list.** Lives in
      [THREAT_MODEL.md § Out-of-scope risks](../THREAT_MODEL.md#out-of-scope-risks)
      (token contract bugs, network-level events, key compromise,
      front-running). Confirm nothing that should now be in-scope — e.g. a
      new external call the contract makes — has been left here by mistake.
- [ ] **Token-failure behavior matches
      [THREAT_MODEL.md § What happens when a token transfer fails](../THREAT_MODEL.md#what-happens-when-a-token-transfer-fails).**
      A malicious or non-conforming token is the main external trust boundary
      this contract has; verify the rollback-on-failure claim against the
      current `transfer` helper in
      [`contract.rs`](../contracts/stream/src/contract.rs) before an auditor
      does.
- [ ] **Read-only views and preview mechanics match
      [THREAT_MODEL.md § Batch reads and cancellation previews](../THREAT_MODEL.md#batch-reads-and-cancellation-previews).**
      `get_streams` bounds batch sizes to 20 to protect host footprint limits
      and neither `get_streams` nor `cancel_preview` mutates state or extends
      TTL. Verify that client applications treat cancellation previews as
      instantaneous estimates subject to ledger time advancement rather than
      guaranteed quotes.
- [ ] **Every `StreamError` variant is accounted for in the threat model or
      the integrator docs.** Lives in
      [`contracts/stream/src/error.rs`](../contracts/stream/src/error.rs).
      The comment on the enum itself states the interface-stability rule
      (codes are never reused or renumbered); confirm no code was added,
      removed, or renumbered without an entry in
      [CHANGELOG.md](../CHANGELOG.md).

## Test coverage

- [ ] **The full suite passes locally and in CI.** Run `cargo test --locked`
      (see [CONTRIBUTING.md](../CONTRIBUTING.md)); CI runs the same command
      on every push and pull request from
      [`.github/workflows/ci.yml`](../.github/workflows/ci.yml). A red or
      flaky suite at audit time means the auditor is reviewing code the test
      suite doesn't actually vouch for.
- [ ] **Functional coverage by area is current.** The module table at the top
      of [`contracts/stream/src/tests/mod.rs`](../contracts/stream/src/tests/mod.rs)
      lists what each test file covers (`creation`, `withdrawal`,
      `cancellation`, `views`, `events`, `storage`, `auth`). A newly added
      entry point or code path should appear in that table's corresponding
      module, not only in `contract.rs`.
- [ ] **Lint and format are clean.** `cargo clippy --locked --all-targets -- -D
      warnings` and `cargo fmt --check`, both run in
      [`.github/workflows/ci.yml`](../.github/workflows/ci.yml). Not a
      security property by itself, but warnings suppressed or silenced
      locally are exactly the kind of thing an auditor flags as a process gap.
- [ ] **Dependency audit is clean, and every suppression is justified.** Run
      `cargo audit --deny warnings`; any ignored advisory must be listed with
      a reason in [`.cargo/AUDIT.md`](../.cargo/AUDIT.md) and
      [`.cargo/audit.toml`](../.cargo/audit.toml), and re-reviewed after the
      last Soroban SDK upgrade per that document's "Updating this config"
      section.

## Interface and build integrity

- [ ] **The published interface snapshot matches the current build.** Lives
      in [`docs/interface.txt`](interface.txt); regenerate with `stellar
      contract inspect` (see
      [README.md § Reading the contract interface](../README.md#reading-the-contract-interface))
      and diff against the committed copy. An auditor reviews the interface
      that ships, not the one from the last snapshot.
- [ ] **Every interface change since the last audit is in
      [CHANGELOG.md](../CHANGELOG.md),** tagged breaking or non-breaking per
      the policy in
      [docs/INTEGRATOR_OPERATIONS.md § Interface Stability Policy](INTEGRATOR_OPERATIONS.md#interface-stability-policy).
      Hand the auditor this changelog span, not just a diff of `contract.rs`,
      so they know which changes are supposed to be behavior-preserving.
- [ ] **A reproducible build matches the WASM intended for deployment.**
      Follow [README.md § Verifying a deployment](../README.md#verifying-a-deployment)
      end to end on a clean checkout. An audit of source that doesn't
      provably match the deployed bytecode only covers half the risk.

## Disclosure and response readiness

- [ ] **The vulnerability disclosure process is live before the audit
      starts.** [SECURITY.md](../SECURITY.md) points reporters at the
      TricklePay docs repository's security policy; confirm that link
      resolves and the acknowledgement/response timelines in SECURITY.md are
      still realistic commitments, since an audit finding is itself a report
      that will flow through this process.
- [ ] **The "no pause, no upgrade" consequence is understood by whoever signs
      off on the audit result.** This is the central fact in
      [THREAT_MODEL.md § No pause mechanism](../THREAT_MODEL.md#no-pause-mechanism):
      a clean audit report does not create a way to contain a
      post-deployment bug. Decide the redeployment plan
      ([docs/INTEGRATOR_OPERATIONS.md § Redeployment Without Upgradeability](INTEGRATOR_OPERATIONS.md#redeployment-without-upgradeability))
      before funds move, not after a finding.
