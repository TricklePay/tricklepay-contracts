# Spec: TricklePay Contract Tooling Improvements

**Status:** Draft  
**Created:** 2026-11-22  
**Owner:** Development Team

## Overview

This spec covers four critical tooling improvements for the TricklePay contracts repository that will enhance developer experience, streamline deployments, and improve CI/CD reliability. These improvements address gaps in TypeScript integration, deployment tracking, example documentation, and Rust toolchain management.

## Background

The TricklePay contracts project currently has several tooling gaps:

1. **Manual argument encoding**: Frontend and backend manually encode contract arguments using `nativeToScVal`, which is error-prone and lacks type safety.
2. **Unstructured deployment records**: Deployments are documented in markdown templates that scripts cannot parse, making verification and tracking difficult.
3. **Scattered examples**: New integrators must piece together contract usage examples from scattered code snippets.
4. **Unclear Rust version requirements**: No manifest declares `rust-version`, and CI only tests current stable, despite requiring Rust 1.84+ for the `wasm32v1-none` target.

## Requirements

### 1. TypeScript Bindings Generation

**Goal**: Generate type-safe TypeScript client bindings from the compiled WASM contract.

**User Story**: As a frontend/backend developer, I want a typed TypeScript client automatically generated from the contract WASM so that I can invoke contract methods with full type safety and eliminate manual argument encoding errors.

**Acceptance Criteria**:
- [ ] A `make bindings` target generates the TypeScript client from the built WASM
- [ ] The generated output is placed in a documented directory (e.g., `bindings/typescript/`)
- [ ] The generated client includes metadata recording the contract WASM hash it was generated from
- [ ] The README explains how frontend and backend projects can consume the generated bindings
- [ ] The bindings include TypeScript interfaces for all contract types, methods, and events
- [ ] Documentation shows migration examples from manual `nativeToScVal` to the generated client

**Technical Approach**:
- Use `stellar contract bindings typescript` command to generate the client
- Generate bindings as part of the build process, after WASM compilation
- Include the WASM SHA-256 hash in the generated output (via comments or metadata file)
- Output structure: `bindings/typescript/` containing generated `.ts` files
- Add npm/yarn integration instructions to README

**Dependencies**:
- Requires Stellar CLI 25.2.0 (already pinned in project)
- Requires successful WASM build (`make wasm`)

**Out of Scope**:
- Publishing bindings to npm (can be added later)
- Bindings for languages other than TypeScript
- Automatic versioning of bindings (initial implementation only)

---

### 2. Machine-Readable Deployment Registry

**Goal**: Create a structured JSON registry of all deployments that scripts and applications can parse.

**User Story**: As a developer or CI script, I want a machine-readable registry of all contract deployments so that I can programmatically verify deployments, query current contract IDs, and audit deployment history.

**Acceptance Criteria**:
- [ ] A `deployments.json` schema is defined and documented
- [ ] The schema includes: network, contract ID, WASM hash, git commit, deployment date, deployer address
- [ ] Initial entries are added for any existing deployments
- [ ] `scripts/verify-deployment.sh` can iterate the file and verify each entry
- [ ] `scripts/deploy.sh` automatically appends new deployments to the registry
- [ ] The README documents the registry format and usage
- [ ] The release process documentation specifies when to update the registry

**JSON Schema**:
```json
{
  "deployments": [
    {
      "network": "testnet",
      "contractId": "C...",
      "wasmHash": "sha256:...",
      "commit": "a1b2c3d",
      "gitTag": "v1.0.0",
      "deployedAt": "2026-11-22T10:30:00Z",
      "deployedBy": "G...",
      "transactionHash": "...",
      "ledgerNumber": 12345,
      "verified": true
    }
  ]
}
```

**Technical Approach**:
- Create `deployments.json` at repository root
- Modify `scripts/deploy.sh` to append deployment records automatically
- Extend `scripts/verify-deployment.sh` to:
  - Accept `--all` flag to verify all entries in `deployments.json`
  - Accept `--network <name>` to verify all deployments on a specific network
  - Report verification status for each entry
- Add JSON schema validation (optional but recommended)

**Dependencies**:
- Requires `jq` for JSON manipulation in shell scripts
- Integrates with existing `scripts/deploy.sh` and `scripts/verify-deployment.sh`

**Migration Path**:
- Keep `DEPLOYMENT_RECORD_TEMPLATE.md` as a human-readable template
- `deployments.json` becomes the source of truth for automation
- Existing deployment records in docs/ should be migrated to initial `deployments.json` entries

---

### 3. Comprehensive Integration Examples Script

**Goal**: Provide a single script with ready-to-run examples for all contract entry points.

**User Story**: As a new integrator, I want a single script that demonstrates every contract function with concrete examples so that I can quickly understand how to interact with the deployed contract.

**Acceptance Criteria**:
- [ ] `scripts/examples.sh` covers every contract entry point and view function
- [ ] The script accepts contract ID and identity as parameters
- [ ] A `--dry-run` mode prints commands without executing them
- [ ] Examples include: `create_stream`, `withdraw`, `withdraw_amount`, `cancel`, `topup_stream`, and all view functions
- [ ] Each example includes inline comments explaining parameters
- [ ] The script works against both testnet and mainnet (via `--network` flag)
- [ ] The README links to `scripts/examples.sh` in the integration guide section
- [ ] Examples use realistic values and demonstrate common patterns (e.g., no-cliff streams, streams with cliffs)

**Script Interface**:
```bash
# Dry-run mode (prints commands)
./scripts/examples.sh --contract-id C... --identity alice --dry-run

# Execute against testnet
./scripts/examples.sh --contract-id C... --identity alice --network testnet

# Execute specific example
./scripts/examples.sh --contract-id C... --identity alice --example create_stream
```

**Examples to Cover**:
1. **Create stream** (no cliff, with cliff, backdated)
2. **Withdraw** (full balance, partial amount, withdraw_many)
3. **Cancel** (before cliff, after cliff, settled calculation)
4. **Top-up** (add funds to active stream)
5. **View functions** (get_stream, withdrawable, vested, locked, progress, status, etc.)
6. **Batch operations** (get_streams, withdrawable_many)

**Technical Approach**:
- Shell script using Stellar CLI (`stellar contract invoke`)
- Template commands with placeholder substitution
- Color-coded output for readability
- Error handling and validation for required parameters
- Integration with `deployments.json` to auto-populate contract IDs

**Dependencies**:
- Stellar CLI 25.2.0
- Valid Stellar identity configured locally
- Access to target network (testnet/mainnet)

---

### 4. Explicit Rust Version Management

**Goal**: Declare and enforce the minimum supported Rust version across the workspace and CI.

**User Story**: As a contributor, I want the required Rust version clearly documented and enforced by CI so that I know which compiler to use and can trust that the contract builds reliably.

**Acceptance Criteria**:
- [ ] `rust-version` is set in `[workspace.package]` in root `Cargo.toml`
- [ ] The version matches the documented minimum (Rust 1.84+ for `wasm32v1-none`)
- [ ] All crate manifests inherit the workspace `rust-version`
- [ ] A dedicated CI job tests with exactly that minimum version
- [ ] The CI job builds and tests the contract successfully
- [ ] The README minimum version declaration matches the manifest
- [ ] CI continues to test against latest stable as well (separate job)

**Cargo.toml Changes**:
```toml
[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.84"  # Required for wasm32v1-none target
license = "MIT"
repository = "https://github.com/tricklepay/tricklepay-contracts"
```

**CI Job Addition** (`.github/workflows/ci.yml`):
```yaml
  test-msrv:
    name: Test with minimum supported Rust version
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@...
      
      - name: Install Rust MSRV
        run: |
          rustup toolchain install 1.84 --profile minimal
          rustup target add wasm32v1-none --toolchain 1.84
      
      - name: Build with MSRV
        run: cargo +1.84 build --locked --release --target wasm32v1-none
      
      - name: Test with MSRV
        run: cargo +1.84 test --locked
```

**Technical Approach**:
- Add `rust-version = "1.84"` to `[workspace.package]` in root `Cargo.toml`
- Contracts inherit it automatically via `rust-version = { workspace = true }`
- Add new CI job `test-msrv` that installs and tests with Rust 1.84 exactly
- Keep existing CI jobs testing with `stable` to catch regressions with newer Rust
- Update README to reference the workspace manifest as source of truth

**Dependencies**:
- None (Rust 1.84 is already the de facto requirement)

**Validation**:
- CI must pass with both Rust 1.84 (MSRV) and stable
- `cargo msrv verify` should confirm compatibility (optional)

---

## Implementation Plan

### Phase 1: TypeScript Bindings (Week 1)
1. Add `bindings` target to Makefile
2. Create `bindings/typescript/` directory structure
3. Generate bindings from WASM with hash metadata
4. Document consumption in README
5. Test bindings in a sample frontend/backend integration

### Phase 2: Deployment Registry (Week 1)
1. Define `deployments.json` schema
2. Create initial entries from known deployments
3. Modify `scripts/deploy.sh` to append to registry
4. Extend `scripts/verify-deployment.sh` for batch verification
5. Update release process documentation

### Phase 3: Examples Script (Week 2)
1. Create `scripts/examples.sh` scaffold
2. Implement dry-run and execution modes
3. Add examples for all entry points and views
4. Test against testnet deployment
5. Document in README integration section

### Phase 4: Rust Version Management (Week 2)
1. Add `rust-version` to workspace manifest
2. Update crate manifests to inherit workspace setting
3. Add `test-msrv` CI job
4. Verify CI passes with both MSRV and stable
5. Update README to match manifest

### Phase 5: Integration & Documentation (Week 3)
1. Cross-reference all four improvements in README
2. Update CONTRIBUTING.md with new workflows
3. Add changelog entries
4. Create GitHub issue templates referencing new tools
5. Final testing and refinement

---

## Testing Strategy

### TypeScript Bindings
- [ ] Bindings generate successfully from WASM
- [ ] Generated types match contract interface
- [ ] Sample TypeScript code compiles with generated bindings
- [ ] All contract methods are covered
- [ ] WASM hash is recorded correctly

### Deployment Registry
- [ ] Schema validation passes for all entries
- [ ] `verify-deployment.sh --all` succeeds for valid entries
- [ ] Invalid entries are detected and reported
- [ ] New deployments are appended correctly
- [ ] JSON format is valid and parseable

### Examples Script
- [ ] Dry-run mode prints all commands correctly
- [ ] Execution mode successfully calls contract
- [ ] All examples cover documented use cases
- [ ] Error messages are clear and helpful
- [ ] Script works on both testnet and mainnet

### Rust Version Management
- [ ] `cargo build` succeeds with Rust 1.84
- [ ] `cargo test` passes with Rust 1.84
- [ ] CI job for MSRV passes
- [ ] CI job for stable continues to pass
- [ ] No version conflicts in dependency tree

---

## Documentation Updates

### README.md
- Add "TypeScript Integration" section linking to bindings
- Document `deployments.json` format and location
- Add "Integration Examples" section referencing `scripts/examples.sh`
- Update "Requirements" section with explicit Rust version
- Add Makefile targets: `make bindings`, `make verify-all-deployments`

### CONTRIBUTING.md
- Document binding generation workflow
- Explain when to update `deployments.json`
- Reference examples script for testing integrations
- Clarify Rust version policy

### docs/INTEGRATOR_OPERATIONS.md
- Link to TypeScript bindings documentation
- Reference deployment registry for contract ID lookup
- Point to examples script as integration starting point

---

## Success Metrics

1. **TypeScript Bindings**: Frontend/backend teams adopt generated client, eliminating manual encoding bugs
2. **Deployment Registry**: All deployments tracked in `deployments.json`, verified automatically
3. **Examples Script**: New integrators cite script as primary learning resource
4. **Rust Version**: CI catches incompatibilities with Rust <1.84 before merging

---

## Future Enhancements (Out of Scope)

- Publish TypeScript bindings to npm registry
- Automated deployment registry updates via CI
- Interactive examples script (TUI)
- Bindings for Rust, Python, Go clients
- Automated MSRV bump detection
- Contract upgrade path with binding versioning

---

## Open Questions

1. Should bindings be checked into git or generated on-demand?
   - **Recommendation**: Check in generated bindings for easier consumption
2. Should `deployments.json` be nested in `deployments/` directory?
   - **Recommendation**: Keep at root for easy script access
3. Should examples script support custom RPC endpoints?
   - **Recommendation**: Yes, add `--rpc-url` flag
4. Should MSRV be Rust 1.84 or higher to include buffer for future fixes?
   - **Recommendation**: Start with 1.84, bump as needed

---

## References

- [Stellar Contract Bindings Documentation](https://developers.stellar.org/docs)
- [Soroban SDK 25.3.2 Release Notes](https://github.com/stellar/rs-soroban-sdk/releases/tag/v25.3.2)
- [Cargo rust-version Documentation](https://doc.rust-lang.org/cargo/reference/manifest.html#the-rust-version-field)
- Current project: `docs/DEPLOYMENT_RECORD_TEMPLATE.md`
- Current project: `README.md` (Rust 1.84+ requirement)
