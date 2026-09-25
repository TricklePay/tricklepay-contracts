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

- **Rust** with the pinned toolchain. The exact version and the `wasm32v1-none`
  target are declared in `rust-toolchain.toml`; install the toolchain and target
  with [rustup](https://rustup.rs).
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
```

> **Tip:** You can use the project's cargo aliases (defined in `.cargo/config.toml`) for shorter commands: `cargo fmt-check`, `cargo lint`, and `cargo test`. These run the exact same checks as the `Makefile` targets.

CI runs the same checks on every push and pull request. The audit command uses the
allowlist in `.cargo/audit.toml`; see the
[Testing section of the README](README.md#testing) for what is ignored and why.

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
   issue you are addressing (for example `Closes #123`).
6. Be responsive to review feedback; follow-up commits during review are fine.

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

## Code of conduct

Be respectful and constructive in all project spaces. See the
[shared contribution guide](https://github.com/TricklePay/docs/blob/main/CONTRIBUTING.md)
for the full expectations.
