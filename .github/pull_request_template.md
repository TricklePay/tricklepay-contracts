## Summary

<!-- Describe what changed and why. -->

## Linked Issue

<!-- Use a closing keyword when possible, for example: Closes #123. -->

Closes #

## Testing

<!-- List the commands you ran, or explain why testing was not run. -->

## Checklist

- [ ] `make check` (or individual checks: `make check-manifest`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`, `cargo audit --deny warnings`, `./scripts/diff-interface.sh`)
- [ ] `make check-manifest` (`cargo metadata --no-deps --locked`)
- [ ] `cargo fmt --check`
- [ ] `cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cargo test --locked`
- [ ] `cargo audit --deny warnings`
- [ ] `./scripts/diff-interface.sh` (interface snapshot matches contract)
- [ ] `cargo build --locked --release --target wasm32v1-none`
- [ ] `make check-markdown-links` (if documentation was modified)
- [ ] `make lint-scripts` (if shell scripts were modified)
- [ ] Regenerated `docs/interface.txt` via `make snapshot` (if the contract interface changed)
- [ ] Added a `CHANGELOG.md` entry (required for contract interface, ABI, or breaking changes)
