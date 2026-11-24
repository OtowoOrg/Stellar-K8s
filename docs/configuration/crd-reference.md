# CRD Reference

The `StellarNode` API is documented in three places, each with a different
audience. Use this page to pick the right one.

| You want to… | Go to |
| --- | --- |
| Look up a single field's type, default, and description | [StellarNode API Reference](../api-reference.md) |
| See the machine-readable schema, e.g. to generate a client | [`config/crd/stellarnode-crd.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/config/crd/stellarnode-crd.yaml) |
| Read the authoritative definition of a field, including ones the schema has not caught up with | The Rust doc comments in `src/crd/stellar_node.rs` |
| Understand how a field affects the generated workload | [Configuration](index.md) and [Operator Environment](operators.md) |

## API Reference

[`docs/api-reference.md`](../api-reference.md) is the field-by-field reference for
`StellarNode`, covering `spec` and `status`.

!!! note "Generated — do not hand-edit"

    This file is generated from the CRD schema and CI verifies it is current. To
    change it, change the Rust types in `src/crd/` and regenerate:

    ```bash
    make generate-api-docs
    ```

    CI runs `make check-api-docs`, which fails if the checked-in file does not
    match the schema.

    The generated reference can only describe what the committed schema declares.
    If you need a field that is missing here, check the Rust doc comments in
    `src/crd/stellar_node.rs` — they are the source of truth the schema is
    generated from, and a field added there will not appear in this page until
    the schema is regenerated.

## Machine-readable schema

The CRDs live in `config/crd/`, one file per custom resource:

| File | Resource |
| --- | --- |
| `stellarnode-crd.yaml` | `StellarNode` |
| `stellaraiops-crd.yaml` | `StellarAIops` |
| `stellarautoscaler-crd.yaml` | `StellarAutoscaler` |
| `stellarbenchmark-crd.yaml` | `StellarBenchmark` |
| `stellarbenchmarkreport-crd.yaml` | `StellarBenchmarkReport` |
| `stellardr-crd.yaml` | `StellarDR` |
| `stellarfederation-crd.yaml` | `StellarFederation` |
| `stellargitopsconfig-crd.yaml` | `StellarGitOpsConfig` |
| `stellarobservability-crd.yaml` | `StellarObservability` |
| `stellarsecuritypolicy-crd.yaml` | `StellarSecurityPolicy` |
| `stellarupgrade-crd.yaml` | `StellarUpgrade` |

`StellarNode` is the resource the operator reconciles; the others are inputs to
its supporting subsystems and are documented alongside the feature that uses
them.

## From field to workload

A field rarely affects the cluster on its own. These are the paths that matter
most in practice:

| Field | Effect on generated resources |
| --- | --- |
| `spec.nodeType` | Selects the workload kind, the data mount path, and the worker-thread variables. See [Storage](storage.md) |
| `spec.storage.*` | The `<node>-data` PVC, its class, size, and retention policy. See [Storage](storage.md) |
| `spec.validatorConfig.quorumSet` | The quorum fragment of the generated `stellar-core.cfg`. See [Configuration](index.md) |
| `spec.validatorConfig.seedSecretRef`, `spec.validatorConfig.seedSecretSource` | Which seed env var is injected. See [Operator Environment](operators.md#seed-environment-variables) |
| `spec.stellarCoreEnv`, `spec.horizonEnv` | Replaces same-named operator-generated container env entries. See [Operator Environment](operators.md#overriding-with-specstellarcoreenv) |
| `spec.volumes`, `spec.volumeMounts` | Extra volumes on the pod — needed for any writable path, since the root filesystem is read-only |
| `spec.historyMode` | Sets the catch-up flags in `stellar-core.cfg` and, absent an explicit `storage.size`, the default PVC size |

## Related

- [Configuration](index.md) — generated ConfigMap keys
- [Operator Environment](operators.md) — pod hardening and env override rules
- [Storage](storage.md) — PVCs and mounts
- [API Versioning](../api-versioning.md)
