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
