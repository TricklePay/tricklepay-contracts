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

CI runs the same checks on every push and pull request. The audit command uses the
allowlist in `.cargo/audit.toml`; see the
[Testing section of the README](README.md#testing) for what is ignored and why.

## How to open a pull request

1. Create a branch from `main`, named after the issue you are working on
   (for example `chore/issue-165`).
2. Make a focused change and run the checks above.
3. Push the branch to your fork and open a pull request against `main`.
4. Describe the change, the motivation, and how you verified it, and link the
   issue you are addressing (for example `Closes #123`).
5. Be responsive to review feedback; follow-up commits during review are fine.

## Code of conduct

Be respectful and constructive in all project spaces. See the
[shared contribution guide](https://github.com/TricklePay/docs/blob/main/CONTRIBUTING.md)
for the full expectations.
