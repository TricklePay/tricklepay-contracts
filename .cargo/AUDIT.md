# Dependency Audit Configuration

This document explains `.cargo/audit.toml`, which configures
[`cargo-audit`](https://github.com/rustsec/rustsec/tree/main/cargo-audit) for
this repository.

## Purpose

Running `cargo audit --deny warnings` in CI would fail on advisories that come
from transitive dependencies pulled in by the Soroban test host. Because those
crates are **never compiled into the deployed WASM** (they are dev/test-only),
ignoring them is safe. The config file lets the CI command stay clean without
passing a growing list of `--ignore` flags on the command line.

## Ignored advisories

| Advisory ID | Crate | Reason ignored |
|---|---|---|
| `RUSTSEC-2024-0388` | `derivative 2.2.0` | Unmaintained; pulled in transitively via `ark-* → soroban-env-host`. Test-host only, not in deployed WASM. |
| `RUSTSEC-2024-0436` | `paste 1.0.15` | Unmaintained; pulled in transitively via `ark-ff`/`wasmi_core → soroban-env-host`. Test-host only, not in deployed WASM. |

These entries should be **reviewed on every Soroban SDK upgrade**. If a newer
SDK version no longer pulls in the affected crate, the corresponding `ignore`
entry should be removed.

## Yanked crates

Yanked-crate warnings are disabled (`[yanked] enabled = false`) because
`spin 0.9.8` is yanked but is a required transitive dependency of
`soroban-wasmi 0.31.1-soroban.20.0.1 → soroban-env-host`. Explicit
vulnerability checks are unaffected by this setting.

## What is NOT suppressed

Vulnerability advisories (i.e., actual CVEs or security bugs) are **never**
ignored by this config. Only "unmaintained" and "yanked" notices for
test-host-only crates are suppressed.

## Updating this config

When a Soroban SDK upgrade changes the transitive dependency tree:

1. Run `cargo audit` locally and inspect any new advisories.
2. Check whether the affected crate is reachable in the deployed WASM
   (`cargo tree --target wasm32-unknown-unknown -p stream`).
3. If it is WASM-reachable, **do not ignore it** — fix the root cause.
4. If it is test-host-only, add a new `ignore` entry with a comment explaining
   the crate, the advisory, and the transitive path.
5. Update the table in this document.
