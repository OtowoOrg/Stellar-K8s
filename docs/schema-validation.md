# Repository-Wide YAML Schema Validation

> **TL;DR** — `schema-validate` checks every manifest under `examples/` and
> `config/samples/` against the OpenAPI v3 schema embedded in this repo's own
> CRDs (`config/crd/*.yaml`). No cluster required. CI fails on any violation.

---

## Overview

Stellar-K8s's CRDs (`StellarNode`, `StellarBenchmark`, ...) embed a full
`openAPIV3Schema` — types, enums, required fields, patterns, numeric bounds.
Historically nothing checked that the 90+ example and sample manifests in
this repository actually satisfy that schema; drift only surfaced when
someone ran `kubectl apply` by hand.

`schema-validate` (`src/bin/schema_validate.rs`, binary name
`schema-validate`) closes that gap:

1. Parses every `config/crd/*.yaml` and extracts `spec.versions[].schema.openAPIV3Schema`.
2. Walks the given search paths (default `examples/`, `config/samples/`),
   parsing every (possibly multi-document) YAML file.
3. For each document with an `apiVersion`/`kind` that matches one of our own
   CRDs, validates its `spec` against that CRD version's schema.
4. Documents whose `kind` isn't one of ours (core Kubernetes types,
   third-party CRDs such as Kafka or PrometheusRule, ...) are silently
   skipped — this tool only knows about schemas it owns.

### Supported schema subset

`type`, `enum`, `properties`, `required`, `additionalProperties` (boolean),
`items`, `minimum`/`maximum`, `minLength`/`maxLength`, `pattern`,
`nullable`, `x-kubernetes-int-or-string`, `x-kubernetes-preserve-unknown-fields`.

**Not evaluated:** `x-kubernetes-validations` (CEL rules) — these require a
CEL interpreter and are validated server-side by the Kubernetes API, not
statically here.

## Running it

```bash
# Default scope: examples/ and config/samples/
cargo run --bin schema-validate
make schema-validate

# A specific file or directory
cargo run --bin schema-validate -- examples/validator-mainnet.yaml

# List every CRD kind/group/version the tool knows about
cargo run --bin schema-validate -- --list
```

It runs as a step in the CI `lint` job and as a local pre-commit hook on any
staged file under `examples/`, `config/samples/`, or `config/crd/`.

## Suppressing intentionally-invalid fixtures

Some checked-in manifests are deliberately invalid — for example
`examples/broken.yaml` and `config/samples/invalid-network-*.yaml` exist to
document manifests the Kubernetes API server is expected to reject. These
are listed by path prefix in `.schema-validate-ignore` at the repo root, one
prefix per line. Add a new entry there (with a comment explaining why) for
any future negative-test fixture rather than leaving it to fail the gate.

## Tests

Unit tests for the validator core live in `src/bin/schema_validate.rs`
(`#[cfg(test)] mod tests`) and cover every supported keyword plus the
ignore-list matching and multi-document YAML parsing. Run with:

```bash
cargo test --bin schema-validate
```
