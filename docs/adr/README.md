# Architecture Decision Records (ADR)

This directory contains Architecture Decision Records (ADRs) for the Stellar-K8s project. ADRs document significant architectural decisions made during the development of the operator.

## What is an ADR?

An Architecture Decision Record (ADR) is a document that captures an important architectural decision made along with its context and consequences. ADRs help:

- **Preserve Context**: Understand why decisions were made
- **Onboard New Contributors**: Quickly understand the system's design rationale
- **Avoid Revisiting Decisions**: Prevent rehashing old discussions
- **Track Evolution**: See how the architecture evolved over time

## Format

We follow the [MADR (Markdown Architecture Decision Record)](https://adr.github.io/madr/) format, which includes:

- **Status**: Proposed, Accepted, Deprecated, Superseded
- **Context**: The issue motivating this decision
- **Decision**: The change being proposed or decided
- **Consequences**: The resulting context after applying the decision
- **Alternatives Considered**: Other options that were evaluated

## Index

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| [0001](0001-wasm-admission-webhook.md) | Wasm-Based Admission Webhook for Custom Validation | Accepted | 2024-02-24 |
| [0002](0002-rust-language-choice.md) | Choice of Rust Programming Language | Superseded by [ADR-001](../adrs/001-rust-operator-architecture.md) | 2024-03-25 |
| [0003](0003-kube-rs-finalizers.md) | Use of kube-rs Finalizers | Superseded by [ADR-001](../adrs/001-rust-operator-architecture.md) | 2024-03-25 |
| [0004](0004-crd-versioning-strategy.md) | CRD Versioning Strategy | Superseded by [ADR-002](../adrs/002-crd-versioning-strategy.md) | 2024-03-25 |
| [0005](0005-reentrancy-guard-middleware.md) | Native Reentrancy Guard Middleware for Soroban | Accepted | 2026-08-31 |

### Foundational architecture ADRs (`docs/adrs/`)

These two records document the project's core technology choices and are the entry point for new contributors. They supersede ADRs 0002 to 0004 above, which described an earlier design that no longer matches the implementation.

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| [001](../adrs/001-rust-operator-architecture.md) | Rust and kube-rs as the Operator Foundation | Accepted | 2026-09-30 |
| [002](../adrs/002-crd-versioning-strategy.md) | CRD API Evolution and Backward Compatibility | Accepted | 2026-09-30 |

## Creating a New ADR

1. Copy the template or use an existing ADR as reference
2. Number it sequentially (e.g., 0002, 0003)
3. Use a descriptive filename: `NNNN-short-title.md`
4. Fill in all sections with relevant information
5. Update this README.md index
6. Submit as part of your PR

## ADR Lifecycle

```
Proposed → Accepted → [Deprecated/Superseded]
```

- **Proposed**: Under discussion, not yet implemented
- **Accepted**: Decision made and implemented
- **Deprecated**: No longer recommended but still in use
- **Superseded**: Replaced by a newer ADR

## Resources

- [MADR Template](https://adr.github.io/madr/)
- [ADR GitHub Organization](https://adr.github.io/)
- [Documenting Architecture Decisions](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
