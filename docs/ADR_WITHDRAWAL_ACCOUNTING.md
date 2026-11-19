# Withdrawal Accounting Decision

## Context

`withdraw_with` records a successful withdrawal with
`stream.withdrawn += amount`. This addition is safe only while the stream
invariants hold: each stream's `total_amount` is capped at `i64::MAX`, and the
selected withdrawal amount cannot exceed the vested amount minus the amount
already withdrawn. Therefore, after a successful selection,
`stream.withdrawn + amount` cannot exceed `total_amount` or overflow `i128`.

## Options and interface effects

1. **Keep `+=` (chosen).** This preserves the existing contract behavior and
   public interface. It relies on the stream amount and withdrawal bounds
   remaining intact. The release profile also keeps overflow checks enabled,
   which issue #493 guards with a local check.
2. **Use `checked_add` and panic on `None`.** This makes the addition's
   overflow behavior independent of the build profile and does not add a
   contract error code, so the public error interface stays unchanged. It does
   introduce an explicit panic path for an invariant violation.
3. **Use `checked_add` and return a new `StreamError`.** This also makes the
   overflow behavior independent of the build profile, but adds a public
   contract error code. Integrators and generated clients may need to update
   their bindings and error handling; the contract interface snapshot must be
   updated as well.

## Decision

Keep the existing `+=` accounting. The capped stream total and withdrawal
selection bound prove the sum stays within range for valid contract state, so
an additional error path is not justified by the current invariant. The release
overflow check remains a safeguard for other arithmetic and for invariant
regressions; it is not the primary proof that this bounded addition is safe.

If future changes weaken either bound or introduce another way to alter
`withdrawn`, revisit this decision. A follow-up should choose explicitly
between a panic on checked-add failure and a new typed error. Tests should
exercise the changed invariant and its failure path; if a typed error is
chosen, update the interface snapshot and client-facing error documentation.
