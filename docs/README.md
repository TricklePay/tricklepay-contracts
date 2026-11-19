# Documentation Index

Welcome to the **TricklePay Contracts** documentation directory. This index helps contributors, integrators, and auditors quickly locate the reference materials relevant to their roles.

| Document | Description | Intended Audience |
| :--- | :--- | :--- |
| [`THREAT_MODEL.md`](../THREAT_MODEL.md) | Assets, trust assumptions, threats, and security boundaries of the contracts. | Security Auditors, Integrators |
| [`CHANGELOG.md`](../CHANGELOG.md) | User-facing record of contract, interface, and tooling changes. | Integrators, Maintainers |
| [`SECURITY.md`](../SECURITY.md) | Security reporting policy and guidance for disclosing vulnerabilities. | Security Researchers, Users |
| [`CONTRIBUTING.md`](../CONTRIBUTING.md) | Development setup, contribution workflow, and review expectations. | Contributors |
| [`AUDIT_READINESS.md`](./AUDIT_READINESS.md) | Checklist, security scope, and preparation guidelines for code audits. | Security Auditors, Core Contributors |
| [`DECISIONS.md`](./DECISIONS.md) | Architectural Decision Records (ADRs) detailing key design choices and rationale. | Contributors, Maintainers |
| [`ADR-552-stream-id-event-topic.md`](./ADR-552-stream-id-event-topic.md) | Decision on whether to index stream ids as event topics, with consumer migration impact. | Contributors, Integrators |
| [`ADR_TIMESTAMP_MAGNITUDE.md`](./ADR_TIMESTAMP_MAGNITUDE.md) | Architectural decision on timestamp magnitude handling and Unix second ranges. | Contributors, Maintainers |
| [`DESIGN_BATCH_STREAM_CREATION.md`](./DESIGN_BATCH_STREAM_CREATION.md) | Proposed atomic batch stream creation API and resource trade-offs. | Contributors, Integrators |
| [`DESIGN_TOPUP.md`](./DESIGN_TOPUP.md) | Design proposal for topping up existing active streams. | Contributors, Integrators |
| [`DESIGN_WITHDRAW_TO.md`](./DESIGN_WITHDRAW_TO.md) | Design proposal for withdrawing stream balances to arbitrary destination addresses. | Contributors, Integrators |
| [`DESIGN_RECIPIENT_TRANSFER.md`](./DESIGN_RECIPIENT_TRANSFER.md) | Design proposal for transferring stream recipient rights. | Contributors, Integrators |
| [`WITHDRAWAL_ACCOUNTING.md`](./WITHDRAWAL_ACCOUNTING.md) | Mathematical principles and invariant proofs of withdrawal accounting. | Auditors, Developers |
| [`INTEGRATOR_OPERATIONS.md`](./INTEGRATOR_OPERATIONS.md) | Operational instructions and integration patterns for third-party consumers. | Integrators, Client Developers |
| [`DEPLOYMENT_RECORD_TEMPLATE.md`](./DEPLOYMENT_RECORD_TEMPLATE.md) | Standardized template for recording deployment metadata, WASM hashes, and contract IDs. | DevOps, Release Managers |
| [`interface.txt`](./interface.txt) | Compiled contract ABI and interface signature definitions. | Integrators, Auditors, Developers |

## Design Documents

Design documents explore architectural changes, new entry points, or major protocol additions prior to implementation. They reside in the `docs/` directory prefixed with `DESIGN_`.

### Status Definitions

- **Proposed**: Under active community and maintainer review. No contract implementation has been merged.
- **Accepted**: Reviewed and approved by maintainers for implementation. Waiting for or undergoing development.
- **Rejected**: Evaluated and decided against. Maintained in the repository to preserve design rationale and prevent revisiting dismissed approaches.
- **Implemented**: Completed, merged into the codebase, and verified by CI.

### Design Document Template

When proposing a new contract feature or interface expansion, use the following template:

```markdown
# Design: <Feature Title>

**Status:** Proposed  
**Related Issues:** #<Issue-Number>  
**Author:** <Author or Team>  
**Date:** YYYY-MM-DD

## Summary

<!-- Brief overview of the proposal and the high-level solution. -->

## Motivation

<!-- Problem statement, user stories, and why existing entry points are insufficient. -->

## Proposed Signature

<!-- Detailed Rust function signatures, contract types, error codes, and constants. -->

## Validation and Effects

<!-- Step-by-step validation order, invariant preservation, state mutations, and event emissions. -->

## Fees and Resource Use

<!-- Comparative gas, CPU/memory instructions, storage footprint, and limits. -->

## Alternatives Considered

<!-- Other designs evaluated and specific technical trade-offs. -->
```
