# Operator SDK Developer Guide

Build custom operators and extensions for Stellar-K8s using the in-tree SDK.

## SDK Components

| Module | Purpose |
|--------|---------|
| `stellar_k8s::sdk` | Public SDK entry point |
| `stellar_k8s::plugin_sdk` | Reconcile hooks and sidecar injectors |
| `stellar_k8s::sdk::codegen` | CRD-to-controller stub generation |
| `stellar_k8s::sdk::testing` | Mock contexts and test harness |

## Scaffold a New Controller

```bash
cargo run --bin stellar-scaffold -- StellarMyResource --print
```

## Example Operators

See `examples/operators/` for three reference implementations:

1. `metrics_exporter.rs` - ReconcileHook
2. `log_shipper.rs` - SidecarInjector
3. `registry_validator.rs` - Admission policy integration via `check_admission`

## Register a Plugin

```rust
use stellar_k8s::plugin_sdk::PluginRegistry;

let registry = std::sync::Arc::new(
    PluginRegistry::new().with_hook(MyHook)
);
```

## CI/CD Template

Use [`.github/workflows/ci.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/ci.yml) as the starting point
for extension CI (lint, test, helm, and security-scan jobs). Reuse composite actions
under `.github/actions/` (especially `setup-rust` and `security-scan`) rather than
copying inline bootstrap.

## Tutorial

See `docs/tutorials/building-your-first-operator.md` for a step-by-step walkthrough.
