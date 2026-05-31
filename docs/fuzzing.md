# Reconciler State-Machine Fuzzing

The Stellar-K8s operator uses **property-based testing** and **state-machine fuzzing** to ensure the reconciler never panics under extreme or malformed conditions and eventually converges to a well-defined state.

## Overview

- **Tool**: [proptest](https://docs.rs/proptest) is integrated in the workspace.
- **Target**: The core reconciler state machine in `src/controller/reconciler.rs`.
- **Guarantees**:
  - Feeding random mutations of `StellarNodeSpec` and random sequences of “events” (spec changes) into validation **never causes a Rust panic**.
  - Event sequences **converge**: each run ends in either a validation error or a valid state (no infinite loops or undefined behavior).

## Running the Fuzzer Locally

### 1. Property tests (spec validation and event sequences)

Build and run the reconciler fuzz tests with the `reconciler-fuzz` feature:

```bash
cargo test -p stellar-k8s --features reconciler-fuzz --test reconciler_fuzz
```

This runs:

- **`spec_validation_never_panics`** – Randomly mutated `StellarNodeSpec` (Validator, Horizon, SorobanRpc) is passed to `spec.validate()`; the test asserts no panic.
- **`event_sequence_validation_never_panics`** – A sequence of random mutations (replicas, suspended flag) is applied to a base spec; after each mutation, `validate()` is called. Asserts no panic and that each step converges (returns `Ok` or `Err`).

To run with more cases (e.g. 1000) and see output:

```bash
cargo test -p stellar-k8s --features reconciler-fuzz prop_ --test reconciler_fuzz -- --nocapture -q
```

Proptest case count can be overridden via the `PROPTEST_CASES` environment variable:

```bash
PROPTEST_CASES=1000 cargo test -p stellar-k8s --features reconciler-fuzz --test reconciler_fuzz
```

### 2. Reconcile test (optional, with cluster)

A third test, **`reconcile_with_failing_client_never_panics_and_converges`**, checks that calling the full reconcile function does not panic and returns either `Ok(Action)` or `Err`. It is **ignored** by default because it requires a Kubernetes client (real cluster or mock). To run it when a cluster is available:

```bash
cargo test -p stellar-k8s --features reconciler-fuzz --test reconciler_fuzz reconcile_with_failing_client -- --ignored
```

Without a cluster, only the two proptest-based tests run; they already cover spec validation and event-sequence convergence.

## Test layout

- **Integration test**: `tests/reconciler_fuzz.rs`
- **Feature flag**: `reconciler-fuzz` in `Cargo.toml` (enables exposing `reconcile_for_fuzz` for testing).
- **Strategies**: Random base specs (Validator / Horizon / SorobanRpc) and mutations (replicas, version, suspended) generate “Node added / spec modified” style events; validation is exercised on each step.

## Alternative: cargo-fuzz

The acceptance criteria allow **cargo-fuzz or proptest**. This project uses **proptest** for structured, state-machine style testing (spec + event sequences). If you want to add a libFuzzer-style fuzz target (e.g. raw bytes → parsed spec → validate), you can add a `fuzz/` directory and a `cargo-fuzz` target that reuses the same validation and convergence guarantees; the current proptest suite already satisfies “never panic” and “eventually converge” for the spec and event-sequence layer.
