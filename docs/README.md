# Documentation Index

Welcome to the **TricklePay Contracts** documentation directory. This index helps contributors, integrators, and auditors quickly locate reference materials, design proposals, and architectural decisions.

## Index

| Document | Description | Intended Audience |
| :--- | :--- | :--- |
| [`THREAT_MODEL.md`](../THREAT_MODEL.md) | Assets, trust assumptions, threats, and security boundaries of the contracts. | Security Auditors, Integrators |
| [`CHANGELOG.md`](../CHANGELOG.md) | User-facing record of contract, interface, and tooling changes. | Integrators, Maintainers |
| [`SECURITY.md`](../SECURITY.md) | Security reporting policy and guidance for disclosing vulnerabilities. | Security Researchers, Users |
| [`CONTRIBUTING.md`](../CONTRIBUTING.md) | Development setup, contribution workflow, and review expectations. | Contributors |
| [`AUDIT_READINESS.md`](./AUDIT_READINESS.md) | Checklist, security scope, and preparation guidelines for code audits. | Security Auditors, Core Contributors |
| [`DECISIONS.md`](./DECISIONS.md) | Central registry of foundational Architectural Decision Records (ADRs). | Contributors, Maintainers |
| [`ADR_STREAM_ID_EVENT_TOPIC.md`](./ADR_STREAM_ID_EVENT_TOPIC.md) | Decision on indexing stream IDs as event topics vs keeping them in event data. | Contributors, Integrators |
| [`ADR_TIMESTAMP_MAGNITUDE.md`](./ADR_TIMESTAMP_MAGNITUDE.md) | Decision not to bound timestamp magnitudes at contract level. | Contributors, Integrators |
| [`ADR_WITHDRAWAL_ACCOUNTING.md`](./ADR_WITHDRAWAL_ACCOUNTING.md) | Decision on bounded withdrawal addition accounting and overflow guarantees. | Contributors, Auditors |
| [`DESIGN_BATCH_STREAM_CREATION.md`](./DESIGN_BATCH_STREAM_CREATION.md) | Proposed atomic batch stream creation API and resource trade-offs. | Contributors, Integrators |
| [`DESIGN_RECIPIENT_TRANSFER.md`](./DESIGN_RECIPIENT_TRANSFER.md) | Proposed stream recipient transfer mechanisms and security considerations. | Contributors, Integrators |
| [`DESIGN_TOPUP.md`](./DESIGN_TOPUP.md) | Design proposal for stream balance top-up operations. | Contributors, Integrators |
| [`DESIGN_WITHDRAW_TO.md`](./DESIGN_WITHDRAW_TO.md) | Design proposal for third-party withdrawal destination redirection. | Contributors, Integrators |
| [`INTEGRATOR_OPERATIONS.md`](./INTEGRATOR_OPERATIONS.md) | Operational instructions and integration patterns for third-party consumers. | Integrators, Client Developers |
| [`DEPLOYMENT_RECORD_TEMPLATE.md`](./DEPLOYMENT_RECORD_TEMPLATE.md) | Standardized template for recording deployment metadata, WASM hashes, and contract IDs. | DevOps, Release Managers |
| [`interface.txt`](./interface.txt) | Compiled contract ABI and interface signature definitions. | Integrators, Auditors, Developers |

## Document Conventions

To maintain a consistent and predictable documentation structure across platforms and operating systems, all decision and design records follow these conventions:

### Document Types & Placement

- **Foundational Architectural Decisions**: Core, cross-cutting architectural decision records that apply repository-wide are maintained centrally in [`DECISIONS.md`](./DECISIONS.md). Each entry is numbered sequentially (`ADR 1`, `ADR 2`, etc.).
- **Standalone Architectural Decisions (ADRs)**: Scoped decision records that evaluate distinct trade-offs or protocol decisions reside as individual markdown documents in `docs/`.
- **Design Proposals**: Documents specifying proposed new features, API additions, data models, or protocol extensions reside as individual markdown documents in `docs/`.

### Naming Scheme (Prefix, Case, Numbering)

1. **Prefix**:
   - Architectural Decision Records use the prefix `ADR_` (e.g., `ADR_TIMESTAMP_MAGNITUDE.md`).
   - Design proposals use the prefix `DESIGN_` (e.g., `DESIGN_BATCH_STREAM_CREATION.md`).
   - General guides, operational runbooks, and templates use descriptive topical names (e.g., `INTEGRATOR_OPERATIONS.md`, `AUDIT_READINESS.md`).
2. **Case**:
   - All filenames must use **`UPPER_SNAKE_CASE`** with a `.md` extension.
   - Mixed-case (e.g., `Decisions.md`) or lower kebab-case (e.g., `adr-552-...`) are prohibited to avoid case-collision issues on case-insensitive filesystems (macOS and Windows) when interacting with case-sensitive environments (Linux / Git).
3. **Numbering**:
   - Standalone ADRs and design documents use descriptive topic names without issue numbers (e.g., `ADR_STREAM_ID_EVENT_TOPIC.md`, `DESIGN_TOPUP.md`). Omitting tracker issue numbers decouples documentation from external issue trackers and keeps file names concise and content-focused.
   - Sequential numbering (`ADR 1`, `ADR 2`, ...) is reserved exclusively for entries in the centralized [`DECISIONS.md`](./DECISIONS.md) registry.
